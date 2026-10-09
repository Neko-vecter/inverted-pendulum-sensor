#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>

#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/queue.h"
#include "freertos/task.h"

#include "esp_event.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "nvs_flash.h"

#include "lwip/inet.h"
#include "lwip/sockets.h"

#include "mt6701.h"
#include "sensor_protocol.h"

#define SAMPLE_PERIOD_US 1000U
#define SAMPLE_STATS_INTERVAL_US 5000000LL
#define CLOCK_SYNC_INTERVAL_US 1000000LL
#define CLOCK_SYNC_STALE_AFTER_US 3000000LL
#define CLOCK_SYNC_MAX_RTT_US 1000000ULL
#define SAMPLE_QUEUE_LENGTH 64U
#define SAMPLE_TASK_STACK_SIZE 4096U
#define UDP_TASK_STACK_SIZE 4096U
#define WIFI_RECONNECT_TASK_STACK_SIZE 3072U
#define SAMPLE_TASK_PRIORITY 18U
#define UDP_TASK_PRIORITY 7U
#define WIFI_RECONNECT_TASK_PRIORITY 5U

#define WIFI_CONNECTED_BIT BIT0
#define WIFI_RECONNECT_BIT BIT1

#define INVALID_ANGLE 0xFFFFU

static const char *TAG = "pendulum_sensor";

static EventGroupHandle_t s_wifi_events;
static QueueHandle_t s_sample_queue;
static TaskHandle_t s_sample_task;
static esp_timer_handle_t s_sample_timer;
static i2c_master_bus_handle_t s_i2c_bus;
static i2c_master_dev_handle_t s_mt6701;
static uint8_t s_station_mac[6];
static struct sockaddr_in s_server_address;
static bool s_network_enabled;

typedef struct {
    bool valid;
    bool request_pending;
    uint32_t next_sequence;
    uint32_t pending_sequence;
    uint64_t pending_t1_us;
    uint64_t server_session_id;
    int64_t offset_us;
    uint32_t round_trip_us;
    int64_t last_response_us;
    int64_t next_request_us;
} clock_sync_state_t;

static clock_sync_state_t s_clock_sync;

static const char *wifi_disconnect_reason_name(uint8_t reason)
{
    switch (reason) {
    case WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT:
        return "4-way handshake timeout; check the WPA2 passphrase/AP security";
    case WIFI_REASON_HANDSHAKE_TIMEOUT:
        return "handshake timeout; check the WPA2 passphrase/AP security";
    case WIFI_REASON_NO_AP_FOUND:
        return "AP not found; check SSID and signal";
    case WIFI_REASON_AUTH_FAIL:
        return "authentication failed; check the AP security settings";
    default:
        return "unclassified reason";
    }
}

static bool wifi_password_is_valid(const char *password, size_t length)
{
    if (length == 0U || (length >= 8U && length < 64U)) {
        return true;
    }
    if (length != 64U) {
        return false;
    }

    for (size_t index = 0; index < length; ++index) {
        const char value = password[index];
        const bool is_digit = value >= '0' && value <= '9';
        const bool is_lower_hex = value >= 'a' && value <= 'f';
        const bool is_upper_hex = value >= 'A' && value <= 'F';
        if (!is_digit && !is_lower_hex && !is_upper_hex) {
            return false;
        }
    }
    return true;
}

static bool wifi_is_connected(void)
{
    return (xEventGroupGetBits(s_wifi_events) & WIFI_CONNECTED_BIT) != 0;
}

static void wifi_event_handler(void *argument,
                               esp_event_base_t event_base,
                               int32_t event_id,
                               void *event_data)
{
    (void)argument;

    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
        xEventGroupSetBits(s_wifi_events, WIFI_RECONNECT_BIT);
        return;
    }

    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
        const wifi_event_sta_disconnected_t *event = event_data;
        xEventGroupClearBits(s_wifi_events, WIFI_CONNECTED_BIT);
        xEventGroupSetBits(s_wifi_events, WIFI_RECONNECT_BIT);
        const uint8_t reason = event != NULL ? event->reason : 0U;
        ESP_LOGW(TAG, "Wi-Fi disconnected (reason=%u: %s); reconnecting",
                 (unsigned)reason, wifi_disconnect_reason_name(reason));
        return;
    }

    if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
        const ip_event_got_ip_t *event = event_data;
        xEventGroupClearBits(s_wifi_events, WIFI_RECONNECT_BIT);
        xEventGroupSetBits(s_wifi_events, WIFI_CONNECTED_BIT);
        ESP_LOGI(TAG, "Wi-Fi ready; node IPv4=" IPSTR, IP2STR(&event->ip_info.ip));
    }
}

static void wifi_reconnect_task(void *argument)
{
    (void)argument;

    for (;;) {
        (void)xEventGroupWaitBits(s_wifi_events,
                                  WIFI_RECONNECT_BIT,
                                  pdTRUE,
                                  pdFALSE,
                                  portMAX_DELAY);
        if (wifi_is_connected()) {
            continue;
        }

        TickType_t retry_delay = pdMS_TO_TICKS(500U);
        const TickType_t max_retry_delay = pdMS_TO_TICKS(8000U);
        while (!wifi_is_connected()) {
            const esp_err_t err = esp_wifi_connect();
            if (err != ESP_OK) {
                ESP_LOGW(TAG, "esp_wifi_connect failed: %s", esp_err_to_name(err));
            }

            (void)xEventGroupWaitBits(s_wifi_events,
                                      WIFI_CONNECTED_BIT,
                                      pdFALSE,
                                      pdFALSE,
                                      retry_delay);
            if (retry_delay < max_retry_delay) {
                retry_delay *= 2U;
                if (retry_delay > max_retry_delay) {
                    retry_delay = max_retry_delay;
                }
            }
        }
    }
}

static esp_err_t wifi_start(void)
{
    const size_t ssid_length = strlen(CONFIG_SENSOR_WIFI_SSID);
    const size_t password_length = strlen(CONFIG_SENSOR_WIFI_PASSWORD);
    if (ssid_length == 0) {
        ESP_LOGW(TAG, "Wi-Fi SSID is empty; sampling will run without UDP");
        return ESP_ERR_INVALID_STATE;
    }
    if (ssid_length > sizeof(((wifi_config_t *)0)->sta.ssid) ||
        password_length > sizeof(((wifi_config_t *)0)->sta.password)) {
        ESP_LOGE(TAG, "Wi-Fi SSID/password exceeds the ESP-IDF field size");
        return ESP_ERR_INVALID_ARG;
    }
    if (!wifi_password_is_valid(CONFIG_SENSOR_WIFI_PASSWORD, password_length)) {
        ESP_LOGE(TAG,
                 "WPA2 password must be 8-63 bytes or a 64-character hexadecimal PSK");
        return ESP_ERR_INVALID_ARG;
    }

    if (inet_pton(AF_INET, CONFIG_SENSOR_SERVER_IPV4,
                  &s_server_address.sin_addr) != 1) {
        ESP_LOGE(TAG, "Invalid server IPv4 address: %s", CONFIG_SENSOR_SERVER_IPV4);
        return ESP_ERR_INVALID_ARG;
    }
    s_server_address.sin_family = AF_INET;
    s_server_address.sin_port = htons(CONFIG_SENSOR_SERVER_PORT);
    ESP_LOGI(TAG, "UDP destination=%s:%u",
             CONFIG_SENSOR_SERVER_IPV4,
             (unsigned)CONFIG_SENSOR_SERVER_PORT);

    ESP_ERROR_CHECK(nvs_flash_init());
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    if (esp_netif_create_default_wifi_sta() == NULL) {
        return ESP_ERR_NO_MEM;
    }

    wifi_init_config_t init_config = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&init_config));
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT,
                                               ESP_EVENT_ANY_ID,
                                               wifi_event_handler,
                                               NULL));
    ESP_ERROR_CHECK(esp_event_handler_register(IP_EVENT,
                                               IP_EVENT_STA_GOT_IP,
                                               wifi_event_handler,
                                               NULL));

    wifi_config_t wifi_config = {0};
    memcpy(wifi_config.sta.ssid, CONFIG_SENSOR_WIFI_SSID, ssid_length);
    memcpy(wifi_config.sta.password, CONFIG_SENSOR_WIFI_PASSWORD, password_length);
    wifi_config.sta.threshold.authmode = password_length == 0
                                             ? WIFI_AUTH_OPEN
                                             : WIFI_AUTH_WPA2_PSK;
    wifi_config.sta.pmf_cfg.capable = true;
    wifi_config.sta.pmf_cfg.required = false;

    ESP_ERROR_CHECK(esp_wifi_set_storage(WIFI_STORAGE_RAM));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &wifi_config));
    ESP_ERROR_CHECK(esp_wifi_set_ps(WIFI_PS_NONE));

    if (xTaskCreate(wifi_reconnect_task,
                    "wifi_reconnect",
                    WIFI_RECONNECT_TASK_STACK_SIZE,
                    NULL,
                    WIFI_RECONNECT_TASK_PRIORITY,
                    NULL) != pdPASS) {
        return ESP_ERR_NO_MEM;
    }

    return esp_wifi_start();
}

static int create_udp_socket(void)
{
    const int socket_fd = socket(AF_INET, SOCK_DGRAM, IPPROTO_IP);
    if (socket_fd < 0) {
        ESP_LOGE(TAG, "socket failed: errno=%d", errno);
        return -1;
    }

    const int flags = fcntl(socket_fd, F_GETFL, 0);
    if (flags < 0 || fcntl(socket_fd, F_SETFL, flags | O_NONBLOCK) < 0) {
        ESP_LOGE(TAG, "Could not make UDP socket nonblocking: errno=%d", errno);
        (void)close(socket_fd);
        return -1;
    }
    return socket_fd;
}

static uint32_t process_time_sync_responses(int socket_fd)
{
    uint32_t updates = 0;
    for (;;) {
        uint8_t packet[128];
        struct sockaddr_in source_address = {0};
        socklen_t source_length = sizeof(source_address);
        const ssize_t packet_length = recvfrom(socket_fd,
                                               packet,
                                               sizeof(packet),
                                               MSG_DONTWAIT,
                                               (struct sockaddr *)&source_address,
                                               &source_length);
        if (packet_length < 0) {
            if (errno != EWOULDBLOCK && errno != EAGAIN) {
                ESP_LOGW(TAG, "Time-sync recvfrom failed: errno=%d", errno);
            }
            break;
        }
        const uint64_t device_t4_us = (uint64_t)esp_timer_get_time();
        if (source_address.sin_family != AF_INET ||
            source_address.sin_addr.s_addr != s_server_address.sin_addr.s_addr ||
            source_address.sin_port != s_server_address.sin_port) {
            continue;
        }

        sensor_time_sync_response_t response = {0};
        if (!sensor_protocol_decode_time_sync_response(packet,
                                                       (size_t)packet_length,
                                                       &response) ||
            memcmp(response.mac, s_station_mac, sizeof(s_station_mac)) != 0 ||
            !s_clock_sync.request_pending ||
            response.sequence != s_clock_sync.pending_sequence ||
            response.device_t1_us != s_clock_sync.pending_t1_us ||
            response.server_session_id == 0U) {
            continue;
        }

        if (device_t4_us < response.device_t1_us ||
            response.server_t3_us < response.server_t2_us) {
            s_clock_sync.request_pending = false;
            continue;
        }

        const uint64_t device_elapsed_us = device_t4_us - response.device_t1_us;
        const uint64_t server_elapsed_us = response.server_t3_us - response.server_t2_us;
        if (device_elapsed_us < server_elapsed_us) {
            s_clock_sync.request_pending = false;
            continue;
        }
        const uint64_t round_trip_us = device_elapsed_us - server_elapsed_us;
        if (round_trip_us > CLOCK_SYNC_MAX_RTT_US) {
            s_clock_sync.request_pending = false;
            continue;
        }

        const int64_t outbound_offset_us = (int64_t)response.server_t2_us -
                                           (int64_t)response.device_t1_us;
        const int64_t inbound_offset_us = (int64_t)response.server_t3_us -
                                          (int64_t)device_t4_us;
        const int64_t measured_offset_us =
            (outbound_offset_us + inbound_offset_us) / 2;

        if (!s_clock_sync.valid ||
            s_clock_sync.server_session_id != response.server_session_id) {
            s_clock_sync.offset_us = measured_offset_us;
        } else {
            s_clock_sync.offset_us +=
                (measured_offset_us - s_clock_sync.offset_us) / 4;
        }
        s_clock_sync.server_session_id = response.server_session_id;
        s_clock_sync.round_trip_us = (uint32_t)round_trip_us;
        s_clock_sync.last_response_us = (int64_t)device_t4_us;
        s_clock_sync.valid = true;
        s_clock_sync.request_pending = false;
        ++updates;
    }
    return updates;
}

static void send_time_sync_request(int socket_fd,
                                   int64_t now_us,
                                   uint32_t *sync_errors_in_interval)
{
    if (now_us < s_clock_sync.next_request_us) {
        return;
    }

    uint8_t packet[SENSOR_PROTOCOL_SYNC_REQUEST_SIZE];
    const uint64_t device_t1_us = (uint64_t)esp_timer_get_time();
    const uint32_t sequence = s_clock_sync.next_sequence++;
    const size_t packet_length = sensor_protocol_encode_time_sync_request(
        s_station_mac, sequence, device_t1_us, packet);
    const ssize_t result = sendto(socket_fd,
                                  packet,
                                  packet_length,
                                  0,
                                  (const struct sockaddr *)&s_server_address,
                                  sizeof(s_server_address));
    if (result == (ssize_t)packet_length) {
        s_clock_sync.pending_sequence = sequence;
        s_clock_sync.pending_t1_us = device_t1_us;
        s_clock_sync.request_pending = true;
    } else {
        ++(*sync_errors_in_interval);
    }
    s_clock_sync.next_request_us = now_us + CLOCK_SYNC_INTERVAL_US;
}

static void udp_sender_task(void *argument)
{
    (void)argument;

    int socket_fd = -1;
    uint32_t sent_in_interval = 0;
    uint32_t errors_in_interval = 0;
    uint32_t sync_errors_in_interval = 0;
    uint32_t sync_updates_in_interval = 0;
    int last_send_errno = 0;
    int64_t interval_start_us = esp_timer_get_time();

    for (;;) {
        if (!wifi_is_connected()) {
            (void)xQueueReset(s_sample_queue);
            s_clock_sync.valid = false;
            s_clock_sync.request_pending = false;
            s_clock_sync.next_request_us = 0;
            vTaskDelay(pdMS_TO_TICKS(50U));
            continue;
        }

        if (socket_fd < 0) {
            socket_fd = create_udp_socket();
            if (socket_fd < 0) {
                vTaskDelay(pdMS_TO_TICKS(1000U));
                continue;
            }
        }

        sync_updates_in_interval += process_time_sync_responses(socket_fd);
        int64_t now_us = esp_timer_get_time();
        if (s_clock_sync.valid &&
            now_us - s_clock_sync.last_response_us > CLOCK_SYNC_STALE_AFTER_US) {
            s_clock_sync.valid = false;
        }
        send_time_sync_request(socket_fd, now_us, &sync_errors_in_interval);

        sensor_sample_t sample;
        if (xQueueReceive(s_sample_queue, &sample, pdMS_TO_TICKS(100U)) != pdTRUE) {
            continue;
        }
        if (!wifi_is_connected()) {
            continue;
        }

        now_us = esp_timer_get_time();
        if (s_clock_sync.valid &&
            now_us - s_clock_sync.last_response_us > CLOCK_SYNC_STALE_AFTER_US) {
            s_clock_sync.valid = false;
        }
        sample.sync_session_id = s_clock_sync.valid
                                     ? s_clock_sync.server_session_id
                                     : 0U;
        sample.clock_offset_us = s_clock_sync.valid ? s_clock_sync.offset_us : 0;
        sample.sync_round_trip_us = s_clock_sync.valid
                                        ? s_clock_sync.round_trip_us
                                        : 0U;
        uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE];
        const size_t packet_length = sensor_protocol_encode_sample(&sample, packet);
        const ssize_t result = sendto(socket_fd,
                                      packet,
                                      packet_length,
                                      0,
                                      (const struct sockaddr *)&s_server_address,
                                      sizeof(s_server_address));
        if (result == (ssize_t)packet_length) {
            ++sent_in_interval;
        } else {
            ++errors_in_interval;
            last_send_errno = result < 0 ? errno : EIO;
        }

        now_us = esp_timer_get_time();
        if (now_us - interval_start_us >= 5000000) {
            ESP_LOGI(TAG,
                     "UDP packets sent=%" PRIu32 ", send errors=%" PRIu32
                     ", last send errno=%d, clock sync updates=%" PRIu32
                     ", sync send errors=%" PRIu32 ", synced=%s"
                     ", offset=%" PRId64 " us, sync RTT=%" PRIu32 " us",
                     sent_in_interval, errors_in_interval, last_send_errno,
                     sync_updates_in_interval, sync_errors_in_interval,
                     s_clock_sync.valid ? "yes" : "no",
                     s_clock_sync.offset_us, s_clock_sync.round_trip_us);
            sent_in_interval = 0;
            errors_in_interval = 0;
            sync_errors_in_interval = 0;
            sync_updates_in_interval = 0;
            last_send_errno = 0;
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
    int64_t stats_interval_start_us = esp_timer_get_time();
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
        sample.timestamp_us = (uint64_t)esp_timer_get_time();
        first_sample = false;

        uint16_t raw_angle = 0;
        if (mt6701_read_angle(s_mt6701, &raw_angle) == ESP_OK) {
            sample.raw_angle = raw_angle;
            sample.status = SENSOR_STATUS_ANGLE_VALID;
        } else {
            sample.raw_angle = INVALID_ANGLE;
            sample.status = SENSOR_STATUS_I2C_ERROR;
            ++i2c_errors_in_interval;
        }
        ++samples_in_interval;

        if (s_network_enabled && wifi_is_connected()) {
            if (xQueueSend(s_sample_queue, &sample, 0) != pdTRUE) {
                ++queue_drops_in_interval;
            }
        } else {
            ++offline_samples_in_interval;
        }

        const int64_t now_us = esp_timer_get_time();
        const int64_t elapsed_us = now_us - stats_interval_start_us;
        if (elapsed_us >= SAMPLE_STATS_INTERVAL_US) {
            const uint32_t sample_rate_hz = (uint32_t)(
                ((uint64_t)samples_in_interval * 1000000ULL) /
                (uint64_t)elapsed_us);
            ESP_LOGI(TAG,
                     "Sampling rate=%" PRIu32 " Hz, reads=%" PRIu32
                     ", I2C errors=%" PRIu32 ", missed timer slots=%" PRIu32
                     ", queue drops=%" PRIu32 ", Wi-Fi offline reads=%" PRIu32,
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
    ESP_LOGI(TAG, "Starting ESP32-C6 MT6701 sensor node");

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

    const esp_err_t sensor_err = mt6701_init(CONFIG_SENSOR_I2C_SDA_GPIO,
                                             CONFIG_SENSOR_I2C_SCL_GPIO,
                                             CONFIG_SENSOR_I2C_CLOCK_HZ,
                                             &s_i2c_bus,
                                             &s_mt6701);
    if (sensor_err != ESP_OK) {
        ESP_LOGE(TAG, "MT6701 I2C setup failed: %s", esp_err_to_name(sensor_err));
        s_mt6701 = NULL;
    }

    s_wifi_events = xEventGroupCreate();
    s_sample_queue = xQueueCreate(SAMPLE_QUEUE_LENGTH, sizeof(sensor_sample_t));
    if (s_wifi_events == NULL || s_sample_queue == NULL) {
        ESP_LOGE(TAG, "Could not allocate task coordination resources");
        return;
    }

    if (!station_mac_valid) {
        ESP_LOGW(TAG, "Network disabled because the station MAC is unavailable");
    } else {
        err = wifi_start();
        if (err == ESP_OK) {
            s_network_enabled = true;
            if (xTaskCreate(udp_sender_task,
                            "udp_sender",
                            UDP_TASK_STACK_SIZE,
                            NULL,
                            UDP_TASK_PRIORITY,
                            NULL) != pdPASS) {
                ESP_LOGE(TAG, "Could not create UDP sender task");
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
             CONFIG_SENSOR_I2C_SDA_GPIO,
             CONFIG_SENSOR_I2C_SCL_GPIO);
}
