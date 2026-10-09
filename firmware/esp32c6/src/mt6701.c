#include "mt6701.h"

#include "driver/gpio.h"

#define MT6701_I2C_ADDRESS 0x06
#define MT6701_ANGLE_HIGH_REGISTER 0x03
#define MT6701_ANGLE_LOW_REGISTER 0x04
#define MT6701_XFER_TIMEOUT_MS 2

esp_err_t mt6701_init(int sda_gpio,
                      int scl_gpio,
                      uint32_t clock_hz,
                      i2c_master_bus_handle_t *bus_handle,
                      i2c_master_dev_handle_t *device_handle)
{
    if (bus_handle == NULL || device_handle == NULL || clock_hz == 0 ||
        !GPIO_IS_VALID_GPIO(sda_gpio) || !GPIO_IS_VALID_GPIO(scl_gpio)) {
        return ESP_ERR_INVALID_ARG;
    }

    const i2c_master_bus_config_t bus_config = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = (gpio_num_t)sda_gpio,
        .scl_io_num = (gpio_num_t)scl_gpio,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
        .flags.enable_internal_pullup = true,
    };

    esp_err_t err = i2c_new_master_bus(&bus_config, bus_handle);
    if (err != ESP_OK) {
        return err;
    }

    const i2c_device_config_t device_config = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = MT6701_I2C_ADDRESS,
        .scl_speed_hz = clock_hz,
    };

    err = i2c_master_bus_add_device(*bus_handle, &device_config, device_handle);
    if (err != ESP_OK) {
        (void)i2c_del_master_bus(*bus_handle);
        *bus_handle = NULL;
    }
    return err;
}

esp_err_t mt6701_read_angle(i2c_master_dev_handle_t device_handle,
                            uint16_t *angle)
{
    if (device_handle == NULL || angle == NULL) {
        return ESP_ERR_INVALID_ARG;
    }

    uint8_t register_address = MT6701_ANGLE_HIGH_REGISTER;
    uint8_t high_byte = 0;
    uint8_t low_byte = 0;

    esp_err_t err = i2c_master_transmit_receive(device_handle,
                                                &register_address,
                                                sizeof(register_address),
                                                &high_byte,
                                                sizeof(high_byte),
                                                MT6701_XFER_TIMEOUT_MS);
    if (err != ESP_OK) {
        return err;
    }

    register_address = MT6701_ANGLE_LOW_REGISTER;
    err = i2c_master_transmit_receive(device_handle,
                                      &register_address,
                                      sizeof(register_address),
                                      &low_byte,
                                      sizeof(low_byte),
                                      MT6701_XFER_TIMEOUT_MS);
    if (err != ESP_OK) {
        return err;
    }

    *angle = ((uint16_t)high_byte << 6) | ((uint16_t)low_byte >> 2);
    return ESP_OK;
}
