/* USER CODE BEGIN Header */
/**
 ******************************************************************************
 * @file           : main.c
 * @brief          : Main program body
 ******************************************************************************
 * @attention
 *
 * Copyright (c) 2025 STMicroelectronics.
 * All rights reserved.
 *
 * This software is licensed under terms that can be found in the LICENSE file
 * in the root directory of this software component.
 * If no LICENSE file comes with this software, it is provided AS-IS.
 *
 ******************************************************************************
 */
/* USER CODE END Header */
/* Includes ------------------------------------------------------------------*/
#include "main.h"
#include "usb_device.h"

/* Private includes ----------------------------------------------------------*/
/* USER CODE BEGIN Includes */
#include "cs43l22.h"
#include "led_pwm.h"
/* USER CODE END Includes */

/* Private typedef -----------------------------------------------------------*/
/* USER CODE BEGIN PTD */

/* USER CODE END PTD */

/* Private define ------------------------------------------------------------*/
/* USER CODE BEGIN PD */
#define AUDIO_BUFFER_SIZE 4096
#define AUDIO_BUFFER_HALF_SIZE (AUDIO_BUFFER_SIZE / 2)
#define INCOMING_BUFFER_SIZE (AUDIO_BUFFER_SIZE * 2)
#define AUDIO_CHANNELS 2
#define BUFFER_TIMEOUT_MS 100

#define ENABLE_REVERB

#ifdef ENABLE_REVERB
#define REVERB_DRY_LEVEL 256
#define REVERB_WET_LEVEL 128
#define REVERB_DECAY 215
#define REVERB_DAMPING 51
#define REVERB_PREDELAY_MS 30
#define REVERB_DIFFUSION 128
#define REVERB_STEREO_SPREAD 23

#define MAX_COMB_SIZE 1600
#define MAX_AP_SIZE 600
#define MAX_PREDELAY 2400
#endif
/* USER CODE END PD */

/* Private macro -------------------------------------------------------------*/
/* USER CODE BEGIN PM */

/* USER CODE END PM */

/* Private variables ---------------------------------------------------------*/
ADC_HandleTypeDef hadc1;

I2C_HandleTypeDef hi2c1;

I2S_HandleTypeDef hi2s3;
DMA_HandleTypeDef hdma_spi3_tx;

SPI_HandleTypeDef hspi1;

TIM_HandleTypeDef htim4;

/* USER CODE BEGIN PV */
volatile int16_t buffer_audio[AUDIO_BUFFER_SIZE * AUDIO_CHANNELS]; // Single audio buffer
volatile uint32_t audio_buffer_w_ptr = 0; // Write pointer
volatile uint32_t last_data_time = 0; // Last time data was received
volatile int16_t incoming_buffer[INCOMING_BUFFER_SIZE * AUDIO_CHANNELS]; // Incoming data buffer
volatile uint32_t incoming_w_ptr = 0; // Incoming write pointer
volatile uint32_t incoming_r_ptr = 0; // Incoming read pointer
volatile int16_t last_L = 0; // Last left sample
volatile int16_t last_R = 0; // Last right sample
// volatile uint8_t is_paused = 0; // Pause state

#ifdef ENABLE_REVERB
// Buffers
int16_t comb_L[4][MAX_COMB_SIZE];
int16_t comb_R[4][MAX_COMB_SIZE];
int16_t ap_L[2][MAX_AP_SIZE];
int16_t ap_R[2][MAX_AP_SIZE];
int16_t predelay_buf_L[MAX_PREDELAY];
int16_t predelay_buf_R[MAX_PREDELAY];

// Indices
uint16_t comb_idx_L[4] = { 0 };
uint16_t comb_idx_R[4] = { 0 };
uint16_t ap_idx_L[2] = { 0 };
uint16_t ap_idx_R[2] = { 0 };
uint16_t pre_idx = 0;

// Filter States
int16_t comb_damp_L[4] = { 0 };
int16_t comb_damp_R[4] = { 0 };

// Tunings
const uint16_t comb_lens[4] = { 1116, 1188, 1277, 1356 };
const uint16_t ap_lens[2] = { 556, 441 };

volatile uint16_t reverb_dry_level = REVERB_DRY_LEVEL;
volatile uint16_t reverb_wet_level = REVERB_WET_LEVEL;
#endif
/* USER CODE END PV */

/* Private function prototypes -----------------------------------------------*/
void SystemClock_Config(void);
static void MX_GPIO_Init(void);
static void MX_DMA_Init(void);
static void MX_I2C1_Init(void);
static void MX_I2S3_Init(void);
static void MX_SPI1_Init(void);
static void MX_ADC1_Init(void);
static void MX_TIM4_Init(void);
/* USER CODE BEGIN PFP */
void ApplyDSP(int16_t in_L, int16_t in_R, int16_t* out_L, int16_t* out_R);
void ProcessAudioChunk(int16_t* output_buffer, uint32_t count);
/* USER CODE END PFP */

/* Private user code ---------------------------------------------------------*/
/* USER CODE BEGIN 0 */

/* USER CODE END 0 */

/**
 * @brief  The application entry point.
 * @retval int
 */
int main(void)
{

    /* USER CODE BEGIN 1 */

    /* USER CODE END 1 */

    /* MCU Configuration--------------------------------------------------------*/

    /* Reset of all peripherals, Initializes the Flash interface and the Systick. */
    HAL_Init();

    /* USER CODE BEGIN Init */

    /* USER CODE END Init */

    /* Configure the system clock */
    SystemClock_Config();

    /* USER CODE BEGIN SysInit */

    /* USER CODE END SysInit */

    /* Initialize all configured peripherals */
    MX_GPIO_Init();
    MX_DMA_Init();
    MX_I2C1_Init();
    MX_I2S3_Init();
    MX_SPI1_Init();
    MX_USB_DEVICE_Init();
    MX_ADC1_Init();
    MX_TIM4_Init();
    /* USER CODE BEGIN 2 */
    LED_PWM_Init(&htim4);

#ifdef ENABLE_REVERB
    HAL_ADC_Init(&hadc1);
#endif

    cs43l22_init();

    memset((void*)buffer_audio, 0, sizeof(buffer_audio));
    memset((void*)incoming_buffer, 0, sizeof(incoming_buffer));
#ifdef ENABLE_REVERB
    memset((void*)comb_L, 0, sizeof(comb_L));
    memset((void*)comb_R, 0, sizeof(comb_R));
    memset((void*)ap_L, 0, sizeof(ap_L));
    memset((void*)ap_R, 0, sizeof(ap_R));
    memset((void*)predelay_buf_L, 0, sizeof(predelay_buf_L));
    memset((void*)predelay_buf_R, 0, sizeof(predelay_buf_R));
#endif

    cs43l22_play((void*)buffer_audio, AUDIO_BUFFER_SIZE * AUDIO_CHANNELS);

    /* USER CODE END 2 */

    /* Infinite loop */
    /* USER CODE BEGIN WHILE */
    while (1) {
        /* USER CODE END WHILE */

        /* USER CODE BEGIN 3 */
        // if (HAL_GPIO_ReadPin(B1_GPIO_Port, B1_Pin) == GPIO_PIN_SET) {
        //     HAL_Delay(50); // Debounce
        //     if (HAL_GPIO_ReadPin(B1_GPIO_Port, B1_Pin) == GPIO_PIN_SET) {
        //         is_paused = !is_paused;
        //         // Wait for release
        //         while (HAL_GPIO_ReadPin(B1_GPIO_Port, B1_Pin) == GPIO_PIN_SET) {
        //             HAL_Delay(10);
        //         }
        //     }
        // }
        // if (HAL_GetTick() - last_data_time > BUFFER_TIMEOUT_MS) {
        //     memset((void*)buffer_audio, 0, sizeof(buffer_audio));
        //     memset((void*)incoming_buffer, 0, sizeof(incoming_buffer));
        // }

#ifdef ENABLE_REVERB
        HAL_ADC_Start(&hadc1);

        HAL_ADC_PollForConversion(&hadc1, 10);
        uint16_t wet_level = HAL_ADC_GetValue(&hadc1);

        HAL_ADC_PollForConversion(&hadc1, 10);
        uint16_t dry_level = HAL_ADC_GetValue(&hadc1);

        HAL_ADC_Stop(&hadc1);

        // Exponential volume scaling to fit human perception
        reverb_wet_level = ((uint32_t)wet_level * wet_level * 256) / 16769025; // 4095 ^ 2
        LED_PWM_SetBrightness(LED_ORANGE, (wet_level * 100) / 4095);
        reverb_dry_level = ((uint32_t)dry_level * dry_level * 256) / 16769025; // 4095 ^ 2
        LED_PWM_SetBrightness(LED_BLUE, (dry_level * 100) / 4095);

        HAL_Delay(1);
#endif
    }
    /* USER CODE END 3 */
}

/**
 * @brief System Clock Configuration
 * @retval None
 */
void SystemClock_Config(void)
{
    RCC_OscInitTypeDef RCC_OscInitStruct = { 0 };
    RCC_ClkInitTypeDef RCC_ClkInitStruct = { 0 };

    /** Configure the main internal regulator output voltage
     */
    __HAL_RCC_PWR_CLK_ENABLE();
    __HAL_PWR_VOLTAGESCALING_CONFIG(PWR_REGULATOR_VOLTAGE_SCALE1);

    /** Initializes the RCC Oscillators according to the specified parameters
     * in the RCC_OscInitTypeDef structure.
     */
    RCC_OscInitStruct.OscillatorType = RCC_OSCILLATORTYPE_HSE;
    RCC_OscInitStruct.HSEState = RCC_HSE_ON;
    RCC_OscInitStruct.PLL.PLLState = RCC_PLL_ON;
    RCC_OscInitStruct.PLL.PLLSource = RCC_PLLSOURCE_HSE;
    RCC_OscInitStruct.PLL.PLLM = 4;
    RCC_OscInitStruct.PLL.PLLN = 144;
    RCC_OscInitStruct.PLL.PLLP = RCC_PLLP_DIV2;
    RCC_OscInitStruct.PLL.PLLQ = 6;
    if (HAL_RCC_OscConfig(&RCC_OscInitStruct) != HAL_OK) {
        Error_Handler();
    }

    /** Initializes the CPU, AHB and APB buses clocks
     */
    RCC_ClkInitStruct.ClockType = RCC_CLOCKTYPE_HCLK | RCC_CLOCKTYPE_SYSCLK
        | RCC_CLOCKTYPE_PCLK1 | RCC_CLOCKTYPE_PCLK2;
    RCC_ClkInitStruct.SYSCLKSource = RCC_SYSCLKSOURCE_PLLCLK;
    RCC_ClkInitStruct.AHBCLKDivider = RCC_SYSCLK_DIV1;
    RCC_ClkInitStruct.APB1CLKDivider = RCC_HCLK_DIV4;
    RCC_ClkInitStruct.APB2CLKDivider = RCC_HCLK_DIV2;

    if (HAL_RCC_ClockConfig(&RCC_ClkInitStruct, FLASH_LATENCY_4) != HAL_OK) {
        Error_Handler();
    }
}

/**
 * @brief ADC1 Initialization Function
 * @param None
 * @retval None
 */
static void MX_ADC1_Init(void)
{

    /* USER CODE BEGIN ADC1_Init 0 */

    /* USER CODE END ADC1_Init 0 */

    ADC_ChannelConfTypeDef sConfig = { 0 };

    /* USER CODE BEGIN ADC1_Init 1 */

    /* USER CODE END ADC1_Init 1 */

    /** Configure the global features of the ADC (Clock, Resolution, Data Alignment and number of conversion)
     */
    hadc1.Instance = ADC1;
    hadc1.Init.ClockPrescaler = ADC_CLOCK_SYNC_PCLK_DIV2;
    hadc1.Init.Resolution = ADC_RESOLUTION_12B;
    hadc1.Init.ScanConvMode = ENABLE;
    hadc1.Init.ContinuousConvMode = DISABLE;
    hadc1.Init.DiscontinuousConvMode = DISABLE;
    hadc1.Init.ExternalTrigConvEdge = ADC_EXTERNALTRIGCONVEDGE_NONE;
    hadc1.Init.ExternalTrigConv = ADC_SOFTWARE_START;
    hadc1.Init.DataAlign = ADC_DATAALIGN_RIGHT;
    hadc1.Init.NbrOfConversion = 2;
    hadc1.Init.DMAContinuousRequests = DISABLE;
    hadc1.Init.EOCSelection = ADC_EOC_SINGLE_CONV;
    if (HAL_ADC_Init(&hadc1) != HAL_OK) {
        Error_Handler();
    }

    /** Configure for the selected ADC regular channel its corresponding rank in the sequencer and its sample time.
     */
    sConfig.Channel = ADC_CHANNEL_1;
    sConfig.Rank = 1;
    sConfig.SamplingTime = ADC_SAMPLETIME_15CYCLES;
    if (HAL_ADC_ConfigChannel(&hadc1, &sConfig) != HAL_OK) {
        Error_Handler();
    }

    /** Configure for the selected ADC regular channel its corresponding rank in the sequencer and its sample time.
     */
    sConfig.Channel = ADC_CHANNEL_2;
    sConfig.Rank = 2;
    if (HAL_ADC_ConfigChannel(&hadc1, &sConfig) != HAL_OK) {
        Error_Handler();
    }
    /* USER CODE BEGIN ADC1_Init 2 */

    /* USER CODE END ADC1_Init 2 */
}

/**
 * @brief I2C1 Initialization Function
 * @param None
 * @retval None
 */
static void MX_I2C1_Init(void)
{

    /* USER CODE BEGIN I2C1_Init 0 */

    /* USER CODE END I2C1_Init 0 */

    /* USER CODE BEGIN I2C1_Init 1 */

    /* USER CODE END I2C1_Init 1 */
    hi2c1.Instance = I2C1;
    hi2c1.Init.ClockSpeed = 100000;
    hi2c1.Init.DutyCycle = I2C_DUTYCYCLE_2;
    hi2c1.Init.OwnAddress1 = 0;
    hi2c1.Init.AddressingMode = I2C_ADDRESSINGMODE_7BIT;
    hi2c1.Init.DualAddressMode = I2C_DUALADDRESS_DISABLE;
    hi2c1.Init.OwnAddress2 = 0;
    hi2c1.Init.GeneralCallMode = I2C_GENERALCALL_DISABLE;
    hi2c1.Init.NoStretchMode = I2C_NOSTRETCH_DISABLE;
    if (HAL_I2C_Init(&hi2c1) != HAL_OK) {
        Error_Handler();
    }
    /* USER CODE BEGIN I2C1_Init 2 */

    /* USER CODE END I2C1_Init 2 */
}

/**
 * @brief I2S3 Initialization Function
 * @param None
 * @retval None
 */
static void MX_I2S3_Init(void)
{

    /* USER CODE BEGIN I2S3_Init 0 */

    /* USER CODE END I2S3_Init 0 */

    /* USER CODE BEGIN I2S3_Init 1 */

    /* USER CODE END I2S3_Init 1 */
    hi2s3.Instance = SPI3;
    hi2s3.Init.Mode = I2S_MODE_MASTER_TX;
    hi2s3.Init.Standard = I2S_STANDARD_PHILIPS;
    hi2s3.Init.DataFormat = I2S_DATAFORMAT_16B;
    hi2s3.Init.MCLKOutput = I2S_MCLKOUTPUT_ENABLE;
    hi2s3.Init.AudioFreq = I2S_AUDIOFREQ_48K;
    hi2s3.Init.CPOL = I2S_CPOL_LOW;
    hi2s3.Init.ClockSource = I2S_CLOCK_PLL;
    hi2s3.Init.FullDuplexMode = I2S_FULLDUPLEXMODE_DISABLE;
    if (HAL_I2S_Init(&hi2s3) != HAL_OK) {
        Error_Handler();
    }
    /* USER CODE BEGIN I2S3_Init 2 */

    /* USER CODE END I2S3_Init 2 */
}

/**
 * @brief SPI1 Initialization Function
 * @param None
 * @retval None
 */
static void MX_SPI1_Init(void)
{

    /* USER CODE BEGIN SPI1_Init 0 */

    /* USER CODE END SPI1_Init 0 */

    /* USER CODE BEGIN SPI1_Init 1 */

    /* USER CODE END SPI1_Init 1 */
    /* SPI1 parameter configuration*/
    hspi1.Instance = SPI1;
    hspi1.Init.Mode = SPI_MODE_MASTER;
    hspi1.Init.Direction = SPI_DIRECTION_2LINES;
    hspi1.Init.DataSize = SPI_DATASIZE_8BIT;
    hspi1.Init.CLKPolarity = SPI_POLARITY_LOW;
    hspi1.Init.CLKPhase = SPI_PHASE_1EDGE;
    hspi1.Init.NSS = SPI_NSS_SOFT;
    hspi1.Init.BaudRatePrescaler = SPI_BAUDRATEPRESCALER_2;
    hspi1.Init.FirstBit = SPI_FIRSTBIT_MSB;
    hspi1.Init.TIMode = SPI_TIMODE_DISABLE;
    hspi1.Init.CRCCalculation = SPI_CRCCALCULATION_DISABLE;
    hspi1.Init.CRCPolynomial = 10;
    if (HAL_SPI_Init(&hspi1) != HAL_OK) {
        Error_Handler();
    }
    /* USER CODE BEGIN SPI1_Init 2 */

    /* USER CODE END SPI1_Init 2 */
}

/**
 * @brief TIM4 Initialization Function
 * @param None
 * @retval None
 */
static void MX_TIM4_Init(void)
{

    /* USER CODE BEGIN TIM4_Init 0 */

    /* USER CODE END TIM4_Init 0 */

    TIM_MasterConfigTypeDef sMasterConfig = { 0 };
    TIM_OC_InitTypeDef sConfigOC = { 0 };

    /* USER CODE BEGIN TIM4_Init 1 */

    /* USER CODE END TIM4_Init 1 */
    htim4.Instance = TIM4;
    htim4.Init.Prescaler = 0;
    htim4.Init.CounterMode = TIM_COUNTERMODE_UP;
    htim4.Init.Period = 65535;
    htim4.Init.ClockDivision = TIM_CLOCKDIVISION_DIV1;
    htim4.Init.AutoReloadPreload = TIM_AUTORELOAD_PRELOAD_DISABLE;
    if (HAL_TIM_PWM_Init(&htim4) != HAL_OK) {
        Error_Handler();
    }
    sMasterConfig.MasterOutputTrigger = TIM_TRGO_RESET;
    sMasterConfig.MasterSlaveMode = TIM_MASTERSLAVEMODE_DISABLE;
    if (HAL_TIMEx_MasterConfigSynchronization(&htim4, &sMasterConfig) != HAL_OK) {
        Error_Handler();
    }
    sConfigOC.OCMode = TIM_OCMODE_PWM1;
    sConfigOC.Pulse = 0;
    sConfigOC.OCPolarity = TIM_OCPOLARITY_HIGH;
    sConfigOC.OCFastMode = TIM_OCFAST_DISABLE;
    if (HAL_TIM_PWM_ConfigChannel(&htim4, &sConfigOC, TIM_CHANNEL_1) != HAL_OK) {
        Error_Handler();
    }
    if (HAL_TIM_PWM_ConfigChannel(&htim4, &sConfigOC, TIM_CHANNEL_2) != HAL_OK) {
        Error_Handler();
    }
    if (HAL_TIM_PWM_ConfigChannel(&htim4, &sConfigOC, TIM_CHANNEL_3) != HAL_OK) {
        Error_Handler();
    }
    if (HAL_TIM_PWM_ConfigChannel(&htim4, &sConfigOC, TIM_CHANNEL_4) != HAL_OK) {
        Error_Handler();
    }
    /* USER CODE BEGIN TIM4_Init 2 */

    /* USER CODE END TIM4_Init 2 */
    HAL_TIM_MspPostInit(&htim4);
}

/**
 * Enable DMA controller clock
 */
static void MX_DMA_Init(void)
{

    /* DMA controller clock enable */
    __HAL_RCC_DMA1_CLK_ENABLE();

    /* DMA interrupt init */
    /* DMA1_Stream5_IRQn interrupt configuration */
    HAL_NVIC_SetPriority(DMA1_Stream5_IRQn, 0, 0);
    HAL_NVIC_EnableIRQ(DMA1_Stream5_IRQn);
}

/**
 * @brief GPIO Initialization Function
 * @param None
 * @retval None
 */
static void MX_GPIO_Init(void)
{
    GPIO_InitTypeDef GPIO_InitStruct = { 0 };
    /* USER CODE BEGIN MX_GPIO_Init_1 */

    /* USER CODE END MX_GPIO_Init_1 */

    /* GPIO Ports Clock Enable */
    __HAL_RCC_GPIOE_CLK_ENABLE();
    __HAL_RCC_GPIOC_CLK_ENABLE();
    __HAL_RCC_GPIOH_CLK_ENABLE();
    __HAL_RCC_GPIOA_CLK_ENABLE();
    __HAL_RCC_GPIOB_CLK_ENABLE();
    __HAL_RCC_GPIOD_CLK_ENABLE();

    /*Configure GPIO pin Output Level */
    HAL_GPIO_WritePin(CS_I2C_SPI_GPIO_Port, CS_I2C_SPI_Pin, GPIO_PIN_RESET);

    /*Configure GPIO pin Output Level */
    HAL_GPIO_WritePin(OTG_FS_PowerSwitchOn_GPIO_Port, OTG_FS_PowerSwitchOn_Pin, GPIO_PIN_SET);

    /*Configure GPIO pin Output Level */
    HAL_GPIO_WritePin(Audio_RST_GPIO_Port, Audio_RST_Pin, GPIO_PIN_RESET);

    /*Configure GPIO pin : CS_I2C_SPI_Pin */
    GPIO_InitStruct.Pin = CS_I2C_SPI_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_OUTPUT_PP;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_LOW;
    HAL_GPIO_Init(CS_I2C_SPI_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : OTG_FS_PowerSwitchOn_Pin */
    GPIO_InitStruct.Pin = OTG_FS_PowerSwitchOn_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_OUTPUT_PP;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_LOW;
    HAL_GPIO_Init(OTG_FS_PowerSwitchOn_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : PDM_OUT_Pin */
    GPIO_InitStruct.Pin = PDM_OUT_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_AF_PP;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_LOW;
    GPIO_InitStruct.Alternate = GPIO_AF5_SPI2;
    HAL_GPIO_Init(PDM_OUT_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : B1_Pin */
    GPIO_InitStruct.Pin = B1_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_EVT_RISING;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    HAL_GPIO_Init(B1_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : BOOT1_Pin */
    GPIO_InitStruct.Pin = BOOT1_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_INPUT;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    HAL_GPIO_Init(BOOT1_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : CLK_IN_Pin */
    GPIO_InitStruct.Pin = CLK_IN_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_AF_PP;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_LOW;
    GPIO_InitStruct.Alternate = GPIO_AF5_SPI2;
    HAL_GPIO_Init(CLK_IN_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : Audio_RST_Pin */
    GPIO_InitStruct.Pin = Audio_RST_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_OUTPUT_PP;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_LOW;
    HAL_GPIO_Init(Audio_RST_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : OTG_FS_OverCurrent_Pin */
    GPIO_InitStruct.Pin = OTG_FS_OverCurrent_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_INPUT;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    HAL_GPIO_Init(OTG_FS_OverCurrent_GPIO_Port, &GPIO_InitStruct);

    /*Configure GPIO pin : MEMS_INT2_Pin */
    GPIO_InitStruct.Pin = MEMS_INT2_Pin;
    GPIO_InitStruct.Mode = GPIO_MODE_EVT_RISING;
    GPIO_InitStruct.Pull = GPIO_NOPULL;
    HAL_GPIO_Init(MEMS_INT2_GPIO_Port, &GPIO_InitStruct);

    /* USER CODE BEGIN MX_GPIO_Init_2 */

    /* USER CODE END MX_GPIO_Init_2 */
}

/* USER CODE BEGIN 4 */
#ifdef ENABLE_REVERB
int16_t CombProcess(int16_t input, int16_t* buffer, uint16_t* idx, uint16_t size, int16_t* damp_state)
{
    int16_t output = buffer[*idx];

    // Damping (Lowpass on feedback)
    *damp_state = ((int32_t)output * (256 - REVERB_DAMPING) + (int32_t)(*damp_state) * REVERB_DAMPING) >> 8;

    int32_t feedback = input + ((*damp_state * REVERB_DECAY) >> 8);

    // Clip
    if (feedback > 32767)
        feedback = 32767;
    if (feedback < -32768)
        feedback = -32768;

    buffer[*idx] = (int16_t)feedback;

    (*idx)++;
    if (*idx >= size)
        *idx = 0;

    return output;
}

int16_t AllpassProcess(int16_t input, int16_t* buffer, uint16_t* idx, uint16_t size)
{
    int16_t buf_out = buffer[*idx];

    int32_t feedback = input + ((buf_out * REVERB_DIFFUSION) >> 8);

    // Clip
    if (feedback > 32767)
        feedback = 32767;
    if (feedback < -32768)
        feedback = -32768;

    buffer[*idx] = (int16_t)feedback;

    int32_t output = buf_out - ((feedback * REVERB_DIFFUSION) >> 8);

    // Clip
    if (output > 32767)
        output = 32767;
    if (output < -32768)
        output = -32768;

    (*idx)++;
    if (*idx >= size)
        *idx = 0;

    return (int16_t)output;
}
#endif

void ApplyDSP(int16_t in_L, int16_t in_R, int16_t* out_L, int16_t* out_R)
{
#ifdef ENABLE_REVERB
    predelay_buf_L[pre_idx] = in_L;
    predelay_buf_R[pre_idx] = in_R;

    // Calculate read index for predelay
    uint16_t pre_read_idx = pre_idx; // 0ms
    uint16_t delay_samples = REVERB_PREDELAY_MS * 48; // 48 samples per ms
    if (delay_samples > MAX_PREDELAY)
        delay_samples = MAX_PREDELAY;

    if (pre_idx >= delay_samples)
        pre_read_idx = pre_idx - delay_samples;
    else
        pre_read_idx = MAX_PREDELAY - (delay_samples - pre_idx);

    int16_t dry_L = in_L;
    int16_t dry_R = in_R;

    int16_t wet_in_L = predelay_buf_L[pre_read_idx];
    int16_t wet_in_R = predelay_buf_R[pre_read_idx];

    pre_idx++;
    if (pre_idx >= MAX_PREDELAY)
        pre_idx = 0;

    int32_t accum_L = 0;
    int32_t accum_R = 0;

    for (int i = 0; i < 4; i++) {
        accum_L += CombProcess(wet_in_L, comb_L[i], &comb_idx_L[i], comb_lens[i], &comb_damp_L[i]);
        accum_R += CombProcess(wet_in_R, comb_R[i], &comb_idx_R[i], comb_lens[i] + REVERB_STEREO_SPREAD, &comb_damp_R[i]);
    }

    // Scale down accumulator to prevent massive clipping
    accum_L /= 4;
    accum_R /= 4;

    int16_t ap_out_L = (int16_t)accum_L;
    int16_t ap_out_R = (int16_t)accum_R;

    for (int i = 0; i < 2; i++) {
        ap_out_L = AllpassProcess(ap_out_L, ap_L[i], &ap_idx_L[i], ap_lens[i]);
        ap_out_R = AllpassProcess(ap_out_R, ap_R[i], &ap_idx_R[i], ap_lens[i] + REVERB_STEREO_SPREAD);
    }

    int32_t final_L = ((dry_L * reverb_dry_level) >> 8) + ((ap_out_L * reverb_wet_level) >> 8);
    int32_t final_R = ((dry_R * reverb_dry_level) >> 8) + ((ap_out_R * reverb_wet_level) >> 8);

    if (final_L > 32767)
        final_L = 32767;
    if (final_L < -32768)
        final_L = -32768;
    if (final_R > 32767)
        final_R = 32767;
    if (final_R < -32768)
        final_R = -32768;

    *out_L = (int16_t)final_L;
    *out_R = (int16_t)final_R;
#else
    *out_L = in_L;
    *out_R = in_R;
#endif
}

void CDC_On_Receive(uint8_t* Buf, uint32_t* Len)
{
    HAL_GPIO_WritePin(GPIOD, GPIO_PIN_12, GPIO_PIN_SET);

    // Validate input length
    if (*Len < 4) {
        HAL_GPIO_WritePin(GPIOD, GPIO_PIN_14, GPIO_PIN_SET);
        return;
    }

    // Truncate to multiple of 4 bytes (stereo frame size)
    *Len &= ~3U;
    uint32_t frames = *Len / 4;

    if (frames == 0) {
        HAL_GPIO_WritePin(GPIOD, GPIO_PIN_14, GPIO_PIN_SET);
        return;
    }

    last_data_time = HAL_GetTick();

    // Calculate available space in circular buffer
    uint32_t available;
    if (incoming_w_ptr >= incoming_r_ptr) {
        available = INCOMING_BUFFER_SIZE - (incoming_w_ptr - incoming_r_ptr) - 1;
    } else {
        available = incoming_r_ptr - incoming_w_ptr - 1;
    }
    if (frames > available) {
        frames = available; // Drop excess data to prevent overwrite
    }

    // Expects Buf to contain interleaved stereo 16-bit samples (L, R, L, R, ...)
    for (uint32_t i = 0; i < frames; i++) {
        incoming_buffer[incoming_w_ptr * AUDIO_CHANNELS] = (int16_t)(Buf[i * 4 + 0] | (Buf[i * 4 + 1] << 8));
        incoming_buffer[incoming_w_ptr * AUDIO_CHANNELS + 1] = (int16_t)(Buf[i * 4 + 2] | (Buf[i * 4 + 3] << 8));
        incoming_w_ptr = (incoming_w_ptr + 1) % INCOMING_BUFFER_SIZE;
    }

    HAL_GPIO_WritePin(GPIOD, GPIO_PIN_14, GPIO_PIN_RESET);
}

void ProcessAudioChunk(int16_t* output_buffer, uint32_t count)
{
    // if (is_paused) {
    //     memset((void*)output_buffer, 0, count * AUDIO_CHANNELS * sizeof(int16_t));
    //     return;
    // }

    for (uint32_t i = 0; i < count; i++) {
        int16_t in_L, in_R;
        if (incoming_r_ptr != incoming_w_ptr) {
            // Data available
            in_L = incoming_buffer[incoming_r_ptr * AUDIO_CHANNELS];
            in_R = incoming_buffer[incoming_r_ptr * AUDIO_CHANNELS + 1];
            incoming_r_ptr = (incoming_r_ptr + 1) % INCOMING_BUFFER_SIZE;
        } else {
            // No data, feed silence
            in_L = 0;
            in_R = 0;
        }

        int16_t out_L, out_R;
        ApplyDSP(in_L, in_R, &out_L, &out_R);

        last_L = out_L;
        last_R = out_R;

        output_buffer[i * AUDIO_CHANNELS] = out_L;
        output_buffer[i * AUDIO_CHANNELS + 1] = out_R;
    }
}

void AUDIO_I2S_TxHalfCpltCallback(void)
{
    ProcessAudioChunk((int16_t*)&buffer_audio[0], AUDIO_BUFFER_HALF_SIZE);
}

void AUDIO_I2S_TxCpltCallback(void)
{
    ProcessAudioChunk((int16_t*)&buffer_audio[AUDIO_BUFFER_HALF_SIZE * AUDIO_CHANNELS], AUDIO_BUFFER_HALF_SIZE);
}
/* USER CODE END 4 */

/**
 * @brief  This function is executed in case of error occurrence.
 * @retval None
 */
void Error_Handler(void)
{
    /* USER CODE BEGIN Error_Handler_Debug */
    /* User can add his own implementation to report the HAL error return state */
    __disable_irq();
    while (1) {
        HAL_GPIO_WritePin(GPIOD, GPIO_PIN_14, GPIO_PIN_SET);
        HAL_Delay(1000);
        HAL_GPIO_WritePin(GPIOD, GPIO_PIN_14, GPIO_PIN_RESET);
        HAL_Delay(1000);
    }
    /* USER CODE END Error_Handler_Debug */
}
#ifdef USE_FULL_ASSERT
/**
 * @brief  Reports the name of the source file and the source line number
 *         where the assert_param error has occurred.
 * @param  file: pointer to the source file name
 * @param  line: assert_param error line source number
 * @retval None
 */
void assert_failed(uint8_t* file, uint32_t line)
{
    /* USER CODE BEGIN 6 */
    /* User can add his own implementation to report the file name and line
       number, ex: printf("Wrong parameters value: file %s on line %d\r\n", file,
       line) */
    /* USER CODE END 6 */
}
#endif /* USE_FULL_ASSERT */
