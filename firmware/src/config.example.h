#pragma once

/* Copy this file to config.h and edit the values for your board/network. */

/* Set to 1 for ESP-NOW, or 0 for Wi-Fi / UDP. */
#define SENSOR_USE_ESPNOW 1

/* ESP-NOW channel; must match the gateway. Valid range: 1-11. */
#define SENSOR_ESPNOW_CHANNEL 1

/* Wi-Fi / UDP settings (used when SENSOR_USE_ESPNOW is 0). */
#define SENSOR_WIFI_SSID ""
#define SENSOR_WIFI_PASSWORD ""
#define SENSOR_SERVER_IPV4 "192.168.1.100"
#define SENSOR_SERVER_PORT 5000
