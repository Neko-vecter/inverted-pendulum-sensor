#pragma once

/* Copy this file to config.h and edit the values for your gateway. */

/* ESP-NOW channel; must match the sensors and server. Valid range: 1-11. */
#define GATEWAY_ESPNOW_CHANNEL 11

/* Set to 1 for native USB Serial/JTAG, or 0 for UART1. */
#define GATEWAY_SERIAL_USB 1

/* UART1 settings (used when GATEWAY_SERIAL_USB is 0). */
#define GATEWAY_UART_TX_GPIO 16
#define GATEWAY_UART_RX_GPIO 17
#define GATEWAY_UART_BAUD 921600
