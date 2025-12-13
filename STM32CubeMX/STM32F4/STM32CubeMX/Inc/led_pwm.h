#ifndef LED_PWM_H
#define LED_PWM_H

#include "stm32f4xx_hal.h"

typedef enum {
    LED_GREEN = 0,
    LED_ORANGE,
    LED_RED,
    LED_BLUE,
    LED_COUNT
} LED_ID;

/**
 * @brief Initialize the LED PWM control.
 * @param htim Pointer to the TIM_HandleTypeDef used for PWM.
 */
void LED_PWM_Init(TIM_HandleTypeDef* htim);

/**
 * @brief Set the LED brightness as a percentage.
 * @param led_id The LED to control.
 * @param brightness_percent Brightness level (0 to 100).
 */
void LED_PWM_SetBrightness(LED_ID led_id, uint8_t brightness_percent);

#endif // LED_PWM_H
