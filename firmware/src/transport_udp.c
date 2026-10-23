#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>
#include "config.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/task.h"
#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "nvs_flash.h"
#include "lwip/inet.h"
#include "lwip/sockets.h"
#include "sensor_transport.h"

#define WIFI_CONNECTED_BIT BIT0
#define WIFI_RECONNECT_BIT BIT1
#define WIFI_RECONNECT_TASK_STACK_SIZE 3072U
#define WIFI_RECONNECT_TASK_PRIORITY 5U
static const char *TAG = "sensor_udp";
static EventGroupHandle_t s_wifi_events;
static struct sockaddr_in s_server_address;
static int s_socket = -1;
static int create_udp_socket(void);

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

bool sensor_transport_ready(void)
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
        if (sensor_transport_ready()) {
            continue;
        }

        TickType_t retry_delay = pdMS_TO_TICKS(500U);
        const TickType_t max_retry_delay = pdMS_TO_TICKS(8000U);
        while (!sensor_transport_ready()) {
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

esp_err_t sensor_transport_start(void)
{
    s_wifi_events = xEventGroupCreate();
    if (s_wifi_events == NULL) {
        return ESP_ERR_NO_MEM;
    }
    const size_t ssid_length = strlen(SENSOR_WIFI_SSID);
    const size_t password_length = strlen(SENSOR_WIFI_PASSWORD);
    if (ssid_length == 0) {
        ESP_LOGW(TAG, "Wi-Fi SSID is empty; sampling will run without UDP");
        return ESP_ERR_INVALID_STATE;
    }
    if (ssid_length > sizeof(((wifi_config_t *)0)->sta.ssid) ||
        password_length > sizeof(((wifi_config_t *)0)->sta.password)) {
        ESP_LOGE(TAG, "Wi-Fi SSID/password exceeds the ESP-IDF field size");
        return ESP_ERR_INVALID_ARG;
    }
    if (!wifi_password_is_valid(SENSOR_WIFI_PASSWORD, password_length)) {
        ESP_LOGE(TAG,
                 "WPA2 password must be 8-63 bytes or a 64-character hexadecimal PSK");
        return ESP_ERR_INVALID_ARG;
    }

    if (inet_pton(AF_INET, SENSOR_SERVER_IPV4,
                  &s_server_address.sin_addr) != 1) {
        ESP_LOGE(TAG, "Invalid server IPv4 address: %s", SENSOR_SERVER_IPV4);
        return ESP_ERR_INVALID_ARG;
    }
    s_server_address.sin_family = AF_INET;
    s_server_address.sin_port = htons(SENSOR_SERVER_PORT);
    ESP_LOGI(TAG, "UDP destination=%s:%u",
             SENSOR_SERVER_IPV4,
             (unsigned)SENSOR_SERVER_PORT);

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
    memcpy(wifi_config.sta.ssid, SENSOR_WIFI_SSID, ssid_length);
    memcpy(wifi_config.sta.password, SENSOR_WIFI_PASSWORD, password_length);
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

    const esp_err_t err = esp_wifi_start();
    if (err != ESP_OK) {
        return err;
    }
    s_socket = create_udp_socket();
    return s_socket < 0 ? ESP_FAIL : ESP_OK;
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

bool sensor_transport_send(const uint8_t *packet, size_t length)
{
    return sendto(s_socket, packet, length, 0,
                  (const struct sockaddr *)&s_server_address,
                  sizeof(s_server_address)) == (ssize_t)length;
}

size_t sensor_transport_receive(uint8_t *packet, size_t capacity, uint32_t *received_us)
{
    struct sockaddr_in source = {0};
    socklen_t source_length = sizeof(source);
    const ssize_t length = recvfrom(s_socket, packet, capacity, MSG_DONTWAIT,
                                    (struct sockaddr *)&source, &source_length);
    *received_us = (uint32_t)esp_timer_get_time();
    if (length < 0) {
        if (errno != EWOULDBLOCK && errno != EAGAIN) {
            ESP_LOGW(TAG, "Time-sync recvfrom failed: errno=%d", errno);
        }
        return 0;
    }
    if (source.sin_family != AF_INET ||
        source.sin_addr.s_addr != s_server_address.sin_addr.s_addr ||
        source.sin_port != s_server_address.sin_port) {
        return 0;
    }
    return (size_t)length;
}
