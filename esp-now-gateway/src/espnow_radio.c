#include "espnow_radio.h"

#include "esp_idf_version.h"
#include "esp_event.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "nvs_flash.h"

static const uint8_t s_broadcast[6] = {255, 255, 255, 255, 255, 255};
static SemaphoreHandle_t s_send_ready;

static esp_err_t configure_broadcast_rate(void)
{
    // Use 802.11n HT20 MCS0 (6.5 Mbps, long guard interval) instead of the
    // ESP-NOW default 1 Mbps 802.11b rate.
    esp_now_rate_config_t rate = {
        .phymode = WIFI_PHY_MODE_HT20,
        .rate = WIFI_PHY_RATE_MCS0_LGI,
        .ersu = false,
        .dcm = false,
    };
    return esp_now_set_peer_rate_config(s_broadcast, &rate);
}

#if ESP_IDF_VERSION >= ESP_IDF_VERSION_VAL(5, 5, 0)
static void send_done(const esp_now_send_info_t *info, esp_now_send_status_t status)
#else
static void send_done(const uint8_t *info, esp_now_send_status_t status)
#endif
{
    (void)info;
    (void)status;
    // Wi-Fi callbacks run in task context. Never block or write serial here.
    (void)xSemaphoreGive(s_send_ready);
}

esp_err_t espnow_radio_start(uint8_t channel, esp_now_recv_cb_t receive_callback)
{
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    ESP_ERROR_CHECK(err);
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    s_send_ready = xSemaphoreCreateBinary();
    if (s_send_ready == NULL) {
        return ESP_ERR_NO_MEM;
    }
    (void)xSemaphoreGive(s_send_ready);

    // ESP-NOW needs the radio driver, but no AP, IP stack, or default netif.
    wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&config));
    ESP_ERROR_CHECK(esp_wifi_set_storage(WIFI_STORAGE_RAM));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_start());
    ESP_ERROR_CHECK(esp_wifi_set_ps(WIFI_PS_NONE));
    ESP_ERROR_CHECK(esp_wifi_set_channel(channel, WIFI_SECOND_CHAN_NONE));
    ESP_ERROR_CHECK(esp_now_init());
    ESP_ERROR_CHECK(esp_now_register_send_cb(send_done));
    ESP_ERROR_CHECK(esp_now_register_recv_cb(receive_callback));
    esp_now_peer_info_t peer = {.channel = channel, .ifidx = WIFI_IF_STA};
    for (size_t i = 0; i < sizeof(s_broadcast); ++i) {
        peer.peer_addr[i] = s_broadcast[i];
    }
    ESP_ERROR_CHECK(esp_now_add_peer(&peer));
    return configure_broadcast_rate();
}

esp_err_t espnow_radio_send(const uint8_t *packet, size_t length)
{
    // Wait for the previous callback before submitting another packet.
    if (xSemaphoreTake(s_send_ready, pdMS_TO_TICKS(20)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    const esp_err_t err = esp_now_send(s_broadcast, packet, length);
    if (err != ESP_OK) {
        (void)xSemaphoreGive(s_send_ready);
    }
    return err;
}
