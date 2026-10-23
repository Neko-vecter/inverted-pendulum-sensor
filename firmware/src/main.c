#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"

#include "esp_log.h"
#include "esp_mac.h"
#include "esp_timer.h"


#include "mt6701.h"
#include "sensor_protocol.h"
#include "sensor_transport.h"

#define SAMPLE_PERIOD_US 1000U
#define SAMPLE_STATS_INTERVAL_US 5000000U
#define CLOCK_SYNC_INTERVAL_US 1000000U
#define CLOCK_SYNC_STALE_AFTER_US 3000000U
#define CLOCK_SYNC_MAX_RTT_US 1000000U
#define COUNTER_HALF_RANGE_US 0x80000000U
#define SAMPLE_QUEUE_LENGTH 64U
#define SAMPLE_TASK_STACK_SIZE 4096U
#define SENDER_TASK_STACK_SIZE 4096U
#define SAMPLE_TASK_PRIORITY 18U
#define SENDER_TASK_PRIORITY 7U


static const char *TAG = "pendulum_sensor";

static QueueHandle_t s_sample_queue;
static TaskHandle_t s_sample_task;
static esp_timer_handle_t s_sample_timer;
static i2c_master_bus_handle_t s_i2c_bus;
static i2c_master_dev_handle_t s_mt6701;
static uint8_t s_station_mac[6];
static bool s_network_enabled;

typedef struct {
    bool valid;
    bool request_pending;
    bool request_schedule_valid;
    uint32_t next_sequence;
    uint32_t pending_sequence;
    uint32_t pending_t1_us;
    uint32_t sync_token;
    uint32_t round_trip_us;
    uint32_t last_response_us;
    uint32_t next_request_us;
} clock_sync_state_t;

static clock_sync_state_t s_clock_sync;

static uint32_t monotonic_counter_us(void)
{
    return (uint32_t)esp_timer_get_time();
}

static bool counter_deadline_reached(uint32_t now_us, uint32_t deadline_us)
{
    return (uint32_t)(now_us - deadline_us) < COUNTER_HALF_RANGE_US;
}

static uint32_t process_time_sync_responses(uint32_t *sync_errors_in_interval)
{
    uint32_t updates = 0;
    for (;;) {
        uint8_t packet[128];
        uint32_t device_t4_us;
        const size_t packet_length = sensor_transport_receive(packet, sizeof(packet),
                                                               &device_t4_us);
        if (packet_length == 0) {
            break;
        }

        sensor_time_sync_response_t response = {0};
        if (!sensor_protocol_decode_time_sync_response(packet,
                                                       (size_t)packet_length,
                                                       &response) ||
            memcmp(response.mac, s_station_mac, sizeof(s_station_mac)) != 0 ||
            !s_clock_sync.request_pending ||
            response.sequence != s_clock_sync.pending_sequence ||
            response.device_t1_us != s_clock_sync.pending_t1_us ||
            response.sync_token == 0U) {
            continue;
        }

        const uint32_t device_elapsed_us = device_t4_us - response.device_t1_us;
        const uint32_t server_elapsed_us = response.server_t3_us - response.server_t2_us;
        if (device_elapsed_us >= COUNTER_HALF_RANGE_US ||
            server_elapsed_us >= COUNTER_HALF_RANGE_US ||
            device_elapsed_us < server_elapsed_us) {
            s_clock_sync.request_pending = false;
            continue;
        }
        const uint32_t round_trip_us = device_elapsed_us - server_elapsed_us;
        if (round_trip_us > CLOCK_SYNC_MAX_RTT_US) {
            s_clock_sync.request_pending = false;
            continue;
        }

        uint8_t report[SENSOR_PROTOCOL_SYNC_REPORT_SIZE];
        const size_t report_length = sensor_protocol_encode_time_sync_report(
            s_station_mac, response.sequence, response.device_t1_us, device_t4_us,
            response.sync_token, report);
        if (report_length == 0U || !sensor_transport_send(report, report_length)) {
            ++(*sync_errors_in_interval);
            s_clock_sync.request_pending = false;
            continue;
        }

        s_clock_sync.sync_token = response.sync_token;
        s_clock_sync.round_trip_us = (uint32_t)round_trip_us;
        s_clock_sync.last_response_us = device_t4_us;
        s_clock_sync.valid = true;
        s_clock_sync.request_pending = false;
        ++updates;
    }
    return updates;
}

static void send_time_sync_request(uint32_t now_us,
                                   uint32_t *sync_errors_in_interval)
{
    if (s_clock_sync.request_schedule_valid &&
        !counter_deadline_reached(now_us, s_clock_sync.next_request_us)) {
        return;
    }

    uint8_t packet[SENSOR_PROTOCOL_SYNC_REQUEST_SIZE];
    const uint32_t device_t1_us = monotonic_counter_us();
    const uint32_t sequence = s_clock_sync.next_sequence++;
    const size_t packet_length = sensor_protocol_encode_time_sync_request(
        s_station_mac, sequence, device_t1_us, packet);
    if (sensor_transport_send(packet, packet_length)) {
        s_clock_sync.pending_sequence = sequence;
        s_clock_sync.pending_t1_us = device_t1_us;
        s_clock_sync.request_pending = true;
    } else {
        ++(*sync_errors_in_interval);
    }
    s_clock_sync.next_request_us = now_us + CLOCK_SYNC_INTERVAL_US;
    s_clock_sync.request_schedule_valid = true;
}

static void sender_task(void *argument)
{
    (void)argument;

    uint32_t sent_in_interval = 0;
    uint32_t errors_in_interval = 0;
    uint32_t sync_errors_in_interval = 0;
    uint32_t sync_updates_in_interval = 0;
    uint32_t interval_start_us = monotonic_counter_us();

    for (;;) {
        if (!sensor_transport_ready()) {
            (void)xQueueReset(s_sample_queue);
            s_clock_sync.valid = false;
            s_clock_sync.request_pending = false;
            s_clock_sync.request_schedule_valid = false;
            vTaskDelay(pdMS_TO_TICKS(50U));
            continue;
        }

        sync_updates_in_interval += process_time_sync_responses(&sync_errors_in_interval);
        uint32_t now_us = monotonic_counter_us();
        if (s_clock_sync.valid &&
            (uint32_t)(now_us - s_clock_sync.last_response_us) >
                CLOCK_SYNC_STALE_AFTER_US) {
            s_clock_sync.valid = false;
        }
        send_time_sync_request(now_us, &sync_errors_in_interval);

        sensor_sample_t sample;
        if (xQueueReceive(s_sample_queue, &sample, pdMS_TO_TICKS(100U)) != pdTRUE) {
            continue;
        }
        if (!sensor_transport_ready()) {
            continue;
        }

        now_us = monotonic_counter_us();
        if (s_clock_sync.valid &&
            (uint32_t)(now_us - s_clock_sync.last_response_us) >
                CLOCK_SYNC_STALE_AFTER_US) {
            s_clock_sync.valid = false;
        }
        sample.sync_token = s_clock_sync.valid ? s_clock_sync.sync_token : 0U;
        uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE];
        const size_t packet_length = sensor_protocol_encode_sample(&sample, packet);
        if (sensor_transport_send(packet, packet_length)) {
            ++sent_in_interval;
        } else {
            ++errors_in_interval;
        }

        now_us = monotonic_counter_us();
        if ((uint32_t)(now_us - interval_start_us) >= SAMPLE_STATS_INTERVAL_US) {
            ESP_LOGI(TAG,
                     "Transport packets submitted=%" PRIu32 ", send errors=%" PRIu32
                     ", clock sync updates=%" PRIu32
                     ", sync send errors=%" PRIu32 ", synced=%s"
                     ", sync token=%" PRIu32 ", sync RTT=%" PRIu32 " us",
                     sent_in_interval, errors_in_interval,
                     sync_updates_in_interval, sync_errors_in_interval,
                     s_clock_sync.valid ? "yes" : "no",
                     s_clock_sync.sync_token, s_clock_sync.round_trip_us);
            sent_in_interval = 0;
            errors_in_interval = 0;
            sync_errors_in_interval = 0;
            sync_updates_in_interval = 0;
            interval_start_us = now_us;
        }
    }
}

static void sampling_task(void *argument)
{
    (void)argument;

    uint32_t sequence = 0;
    uint32_t samples_in_interval = 0;
    uint32_t i2c_errors_in_interval = 0;
    uint32_t missed_slots_in_interval = 0;
    uint32_t queue_drops_in_interval = 0;
    uint32_t offline_samples_in_interval = 0;
    uint32_t stats_interval_start_us = monotonic_counter_us();
    bool first_sample = true;
    for (;;) {
        const uint32_t pending_ticks = ulTaskNotifyTake(pdTRUE, portMAX_DELAY);
        if (pending_ticks > 1U) {
            const uint32_t missed_slots = pending_ticks - 1U;
            missed_slots_in_interval += missed_slots;
            if (!first_sample) {
                sequence += missed_slots;
            }
        }

        sensor_sample_t sample = {0};
        memcpy(sample.mac, s_station_mac, sizeof(sample.mac));
        sample.sequence = sequence++;
        sample.timestamp_us = monotonic_counter_us();
        first_sample = false;

        uint16_t raw_angle = 0;
        if (mt6701_read_angle(s_mt6701, &raw_angle) == ESP_OK) {
            sample.raw_angle = raw_angle;
            sample.status = SENSOR_STATUS_ANGLE_VALID;
        } else {
            sample.raw_angle = 0U;
            sample.status = SENSOR_STATUS_I2C_ERROR;
            ++i2c_errors_in_interval;
        }
        ++samples_in_interval;

        if (s_network_enabled && sensor_transport_ready()) {
            if (xQueueSend(s_sample_queue, &sample, 0) != pdTRUE) {
                ++queue_drops_in_interval;
            }
        } else {
            ++offline_samples_in_interval;
        }

        const uint32_t now_us = monotonic_counter_us();
        const uint32_t elapsed_us = now_us - stats_interval_start_us;
        if (elapsed_us >= SAMPLE_STATS_INTERVAL_US) {
            const uint32_t sample_rate_hz = (uint32_t)(
                ((uint64_t)samples_in_interval * 1000000ULL) / elapsed_us);
            ESP_LOGI(TAG,
                     "Sampling rate=%" PRIu32 " Hz, reads=%" PRIu32
                     ", I2C errors=%" PRIu32 ", missed timer slots=%" PRIu32
                     ", queue drops=%" PRIu32 ", Transport offline reads=%" PRIu32,
                     sample_rate_hz,
                     samples_in_interval,
                     i2c_errors_in_interval,
                     missed_slots_in_interval,
                     queue_drops_in_interval,
                     offline_samples_in_interval);
            samples_in_interval = 0;
            i2c_errors_in_interval = 0;
            missed_slots_in_interval = 0;
            queue_drops_in_interval = 0;
            offline_samples_in_interval = 0;
            stats_interval_start_us = now_us;
        }
    }
}

static void sample_timer_callback(void *argument)
{
    TaskHandle_t task = (TaskHandle_t)argument;
    (void)xTaskNotifyGive(task);
}

void app_main(void)
{
    ESP_LOGI(TAG, "Starting ESP32 MT6701 sensor node");

    esp_err_t err = esp_read_mac(s_station_mac, ESP_MAC_WIFI_STA);
    const bool station_mac_valid = err == ESP_OK;
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "Could not read Wi-Fi station MAC: %s", esp_err_to_name(err));
        memset(s_station_mac, 0, sizeof(s_station_mac));
    } else {
        ESP_LOGI(TAG, "Station MAC=%02X:%02X:%02X:%02X:%02X:%02X",
                 (unsigned)s_station_mac[0], (unsigned)s_station_mac[1],
                 (unsigned)s_station_mac[2], (unsigned)s_station_mac[3],
                 (unsigned)s_station_mac[4], (unsigned)s_station_mac[5]);
    }

    const esp_err_t sensor_err = mt6701_init(SENSOR_I2C_SDA_GPIO,
                                             SENSOR_I2C_SCL_GPIO,
                                             SENSOR_I2C_CLOCK_HZ,
                                             &s_i2c_bus,
                                             &s_mt6701);
    if (sensor_err != ESP_OK) {
        ESP_LOGE(TAG, "MT6701 I2C setup failed: %s", esp_err_to_name(sensor_err));
        s_mt6701 = NULL;
    }

    s_sample_queue = xQueueCreate(SAMPLE_QUEUE_LENGTH, sizeof(sensor_sample_t));
    if (s_sample_queue == NULL) {
        ESP_LOGE(TAG, "Could not allocate task coordination resources");
        return;
    }

    if (!station_mac_valid) {
        ESP_LOGW(TAG, "Network disabled because the station MAC is unavailable");
    } else {
        err = sensor_transport_start();
        if (err == ESP_OK) {
            s_network_enabled = true;
            if (xTaskCreate(sender_task,
                            "sensor_sender",
                            SENDER_TASK_STACK_SIZE,
                            NULL,
                            SENDER_TASK_PRIORITY,
                            NULL) != pdPASS) {
                ESP_LOGE(TAG, "Could not create sender task");
                s_network_enabled = false;
            }
        } else {
            ESP_LOGW(TAG, "Network is disabled: %s", esp_err_to_name(err));
        }
    }

    if (xTaskCreate(sampling_task,
                    "sensor_sample",
                    SAMPLE_TASK_STACK_SIZE,
                    NULL,
                    SAMPLE_TASK_PRIORITY,
                    &s_sample_task) != pdPASS) {
        ESP_LOGE(TAG, "Could not create sampling task");
        return;
    }

    const esp_timer_create_args_t timer_args = {
        .callback = sample_timer_callback,
        .arg = s_sample_task,
        .dispatch_method = ESP_TIMER_TASK,
        .name = "sensor_1khz",
    };
    ESP_ERROR_CHECK(esp_timer_create(&timer_args, &s_sample_timer));
    ESP_ERROR_CHECK(esp_timer_start_periodic(s_sample_timer, SAMPLE_PERIOD_US));
    ESP_LOGI(TAG, "Sampling started: %u Hz target; I2C SDA=%d SCL=%d",
             1000000U / SAMPLE_PERIOD_US,
             SENSOR_I2C_SDA_GPIO,
             SENSOR_I2C_SCL_GPIO);
}
