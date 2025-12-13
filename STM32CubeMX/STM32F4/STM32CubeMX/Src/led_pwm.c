#include "led_pwm.h"

static TIM_HandleTypeDef* pwm_timer;

static const uint32_t led_channels[LED_COUNT] = {
    TIM_CHANNEL_1, // LED_GREEN
    TIM_CHANNEL_2, // LED_ORANGE
    TIM_CHANNEL_3, // LED_RED
    TIM_CHANNEL_4 // LED_BLUE
};

/**
 * @brief Initialize the LED PWM control.
 * @param htim Pointer to the TIM_HandleTypeDef used for PWM.
 */
void LED_PWM_Init(TIM_HandleTypeDef* htim)
{
    pwm_timer = htim;
    for (int i = 0; i < LED_COUNT; i++) {
        HAL_TIM_PWM_Start(pwm_timer, led_channels[i]);
    }
}

/**
 * @brief Set the LED brightness as a percentage.
 * @param led_id The LED to control.
 * @param brightness_percent Brightness level (0 to 100).
 */
void LED_PWM_SetBrightness(LED_ID led_id, uint8_t brightness_percent)
{
    if (led_id >= LED_COUNT) {
        return;
    }

    if (brightness_percent > 100) {
        brightness_percent = 100;
    }

    // Get the Auto-Reload Register (ARR) value which determines the period
    uint32_t arr = __HAL_TIM_GET_AUTORELOAD(pwm_timer);

    // Calculate the Capture Compare Register (CCR) value based on percentage
    // We use 32-bit arithmetic to avoid overflow before division
    uint32_t ccr = (brightness_percent * arr) / 100;

    __HAL_TIM_SET_COMPARE(pwm_timer, led_channels[led_id], ccr);
}
