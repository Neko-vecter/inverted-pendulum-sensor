#include <inttypes.h>
#include <string.h>
#include "sdkconfig.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_timer.h"
#include "espnow_radio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "sensor_protocol.h"
#include "serial_frame.h"
#if CONFIG_GATEWAY_SERIAL_USB
#include "driver/usb_serial_jtag.h"
#if CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG || CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG
#error "Disable the USB primary/secondary console before using USB for binary gateway data"
#endif
#else
#include "driver/uart.h"
#endif

static const char *TAG = "espnow_gateway";
static QueueHandle_t s_packets;
static portMUX_TYPE s_stats_lock = portMUX_INITIALIZER_UNLOCKED;
static uint32_t s_queue_drops;

typedef struct {
    uint8_t data[SERIAL_PAYLOAD_MAX];
    size_t length;
} gateway_packet_t;

static int serial_read(uint8_t *data, size_t length)
{
#if CONFIG_GATEWAY_SERIAL_USB
    return usb_serial_jtag_read_bytes(data, length, pdMS_TO_TICKS(20));
#else
    return uart_read_bytes(UART_NUM_1, data, length, pdMS_TO_TICKS(20));
#endif
}

static bool serial_write(const uint8_t *data, size_t length)
{
#if CONFIG_GATEWAY_SERIAL_USB
    return usb_serial_jtag_write_bytes(data, length, pdMS_TO_TICKS(20)) == (int)length;
#else
    return uart_write_bytes(UART_NUM_1, data, length) == (int)length;
#endif
}

static void serial_start(void)
{
#if CONFIG_GATEWAY_SERIAL_USB
    usb_serial_jtag_driver_config_t config = {
        .tx_buffer_size = 8192, .rx_buffer_size = 1024,
    };
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&config));
#else
    const uart_config_t config = {
        .baud_rate = CONFIG_GATEWAY_UART_BAUD,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };
    ESP_ERROR_CHECK(uart_param_config(UART_NUM_1, &config));
    ESP_ERROR_CHECK(uart_set_pin(UART_NUM_1, CONFIG_GATEWAY_UART_TX_GPIO,
                               CONFIG_GATEWAY_UART_RX_GPIO,
                               UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE));
    ESP_ERROR_CHECK(uart_driver_install(UART_NUM_1, 1024, 8192, 0, NULL, 0));
#endif
}

static void receive_packet(const esp_now_recv_info_t *info, const uint8_t *data, int length)
{
    if (info == NULL || info->src_addr == NULL || data == NULL ||
        (length != SENSOR_PROTOCOL_PACKET_SIZE &&
         length != SENSOR_PROTOCOL_SYNC_REQUEST_SIZE &&
         length != SENSOR_PROTOCOL_SYNC_REPORT_SIZE) ||
        data[0] != 0x49 || data[1] != 0x50 || data[2] != SENSOR_PROTOCOL_VERSION ||
        !((data[3] == SENSOR_PROTOCOL_TYPE_SAMPLE && length == SENSOR_PROTOCOL_PACKET_SIZE) ||
          (data[3] == SENSOR_PROTOCOL_TYPE_TIME_SYNC_REQUEST && length == SENSOR_PROTOCOL_SYNC_REQUEST_SIZE) ||
          (data[3] == SENSOR_PROTOCOL_TYPE_TIME_SYNC_REPORT && length == SENSOR_PROTOCOL_SYNC_REPORT_SIZE)) ||
        memcmp(info->src_addr, data + 4, 6) != 0) {
        return;
    }
    gateway_packet_t packet = {.length = (size_t)length};
    memcpy(packet.data, data, packet.length);
    if (xQueueSend(s_packets, &packet, 0) != pdTRUE) {
        portENTER_CRITICAL(&s_stats_lock);
        ++s_queue_drops;
        portEXIT_CRITICAL(&s_stats_lock);
    }
}

static void forward_to_host(void *argument)
{
    (void)argument;
    uint32_t forwarded = 0, serial_errors = 0;
    int64_t interval_start = esp_timer_get_time();
    for (;;) {
        gateway_packet_t packet;
        if (xQueueReceive(s_packets, &packet, pdMS_TO_TICKS(100)) == pdTRUE) {
            uint8_t frame[SERIAL_FRAME_MAX];
            const size_t length = serial_frame_encode(packet.data, packet.length, frame);
            if (serial_write(frame, length)) {
                ++forwarded;
            } else {
                ++serial_errors;
            }
        }
        const int64_t now = esp_timer_get_time();
        if (now - interval_start >= 5000000) {
            portENTER_CRITICAL(&s_stats_lock);
            const uint32_t drops = s_queue_drops;
            s_queue_drops = 0;
            portEXIT_CRITICAL(&s_stats_lock);
            ESP_LOGI(TAG, "Forwarded=%" PRIu32 ", serial errors=%" PRIu32
                     ", queue drops=%" PRIu32, forwarded, serial_errors, drops);
            forwarded = serial_errors = 0;
            interval_start = now;
        }
    }
}

static void forward_to_nodes(void *argument)
{
    (void)argument;
    uint8_t input[128], encoded[SERIAL_ENCODED_MAX];
    size_t used = 0;
    bool overflow = false;
    for (;;) {
        const int length = serial_read(input, sizeof(input));
        for (int i = 0; i < length; ++i) {
            if (input[i] != 0) {
                if (used < sizeof(encoded)) {
                    encoded[used++] = input[i];
                } else {
                    overflow = true;
                }
                continue;
            }
            uint8_t packet[SERIAL_PAYLOAD_MAX];
            const size_t size = overflow ? 0 : serial_frame_decode(encoded, used, packet);
            used = 0;
            overflow = false;
            if (size == SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE &&
                packet[0] == 0x49 && packet[1] == 0x50 &&
                packet[2] == SENSOR_PROTOCOL_VERSION &&
                packet[3] == SENSOR_PROTOCOL_TYPE_TIME_SYNC_RESPONSE) {
                const esp_err_t err = espnow_radio_send(packet, size);
                if (err != ESP_OK) {
                    ESP_LOGW(TAG, "Time-sync forward failed: %s", esp_err_to_name(err));
                }
            }
        }
    }
}

void app_main(void)
{
    s_packets = xQueueCreate(128, sizeof(gateway_packet_t));
    ESP_ERROR_CHECK(s_packets == NULL ? ESP_ERR_NO_MEM : ESP_OK);
    serial_start();
    ESP_ERROR_CHECK(espnow_radio_start(CONFIG_GATEWAY_ESPNOW_CHANNEL, receive_packet));
    uint8_t mac[6];
    ESP_ERROR_CHECK(esp_read_mac(mac, ESP_MAC_WIFI_STA));
    ESP_LOGI(TAG, "Gateway MAC=%02X:%02X:%02X:%02X:%02X:%02X, channel=%d",
             mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], CONFIG_GATEWAY_ESPNOW_CHANNEL);
    ESP_ERROR_CHECK(xTaskCreate(forward_to_host, "serial_tx", 4096, NULL, 7, NULL)
                    == pdPASS ? ESP_OK : ESP_ERR_NO_MEM);
    ESP_ERROR_CHECK(xTaskCreate(forward_to_nodes, "serial_rx", 4096, NULL, 7, NULL)
                    == pdPASS ? ESP_OK : ESP_ERR_NO_MEM);
}
