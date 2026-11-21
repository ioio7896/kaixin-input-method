//! Shared visual language for the egui-based settings and companion tools.
//! Supports 5 distinct UI design styles with Style 5 (Tactile Industrial) as the default.

use egui::{Color32, FontId, Frame, Margin, RichText, Stroke, TextStyle, Ui};
use std::sync::atomic::{AtomicU8, Ordering};

pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;
pub const CONTROL_HEIGHT: f32 = 32.0;
pub const CONTROL_RADIUS: f32 = 4.0;
pub const CARD_RADIUS: f32 = 6.0;
pub const FLOATING_RADIUS: f32 = 999.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum UiDesignStyle {
    /// Style 1: Windows 11 Fluent 2 Mica & Layered Cards
    Win11Mica = 0,
    /// Style 2: macOS Sequoia / Apple Bento Grid
    MacosBento = 1,
    /// Style 3: Linear / Raycast Pro Dark Utility
    LinearDark = 2,
    /// Style 4: Modern Oriental Ink & Warm Rice Paper
    OrientalInk = 3,
    /// Style 5 (Default): Teenage Engineering Tactile Industrial Minimalist
    #[default]
    TactileIndustrial = 4,
}

static ACTIVE_DESIGN_STYLE: AtomicU8 = AtomicU8::new(UiDesignStyle::TactileIndustrial as u8);

impl UiDesignStyle {
    pub const ALL: [Self; 5] = [
        Self::TactileIndustrial,
        Self::Win11Mica,
        Self::MacosBento,
        Self::LinearDark,
        Self::OrientalInk,
    ];

    pub fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "win11_mica" | "win11" | "mica" | "fluent" | "style1" | "1" => Self::Win11Mica,
            "macos_bento" | "bento" | "macos" | "apple" | "style2" | "2" => Self::MacosBento,
            "linear_dark" | "linear" | "raycast" | "pro_dark" | "style3" | "3" => Self::LinearDark,
            "oriental_ink" | "ink" | "paper" | "oriental" | "style4" | "4" => Self::OrientalInk,
            _ => Self::TactileIndustrial,
        }
    }

    pub fn as_config(self) -> &'static str {
        match self {
            Self::Win11Mica => "win11_mica",
            Self::MacosBento => "macos_bento",
            Self::LinearDark => "linear_dark",
            Self::OrientalInk => "oriental_ink",
            Self::TactileIndustrial => "tactile",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TactileIndustrial => "风格 5 · 复古精密工业硬件风（默认）",
            Self::Win11Mica => "风格 1 · Win11 Fluent 云母原生风",
            Self::MacosBento => "风格 2 · Apple Bento 模块便当盒风",
            Self::LinearDark => "风格 3 · Linear 极客暗黑生产力风",
            Self::OrientalInk => "风格 4 · 东方宣纸水墨雅致人文风",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::TactileIndustrial => "工业硬件风",
            Self::Win11Mica => "Win11 云母",
            Self::MacosBento => "Apple Bento",
            Self::LinearDark => "Linear 暗黑",
            Self::OrientalInk => "宣纸水墨",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::TactileIndustrial => "Teenage Engineering 阳极铝面板 · 工业橙点缀 · 机械键帽质感",
            Self::Win11Mica => "Windows 11 云母暖灰侧栏 · 纯净分层卡片 · 胶囊高亮条",
            Self::MacosBento => "12px 大圆角便当盒网格 · 柔和高亮 · 苹果人机界面质感",
            Self::LinearDark => "深曜石黑底 · 1px 锐利微边框 · 霓虹青高密度极客控制台",
            Self::OrientalInk => "温润宣纸米白 · 浓墨灰字 · 朱砂印章与玉石青东方美学",
        }
    }

    pub fn control_radius(self) -> f32 {
        match self {
            Self::Win11Mica => 4.0,
            Self::MacosBento => 8.0,
            Self::LinearDark => 5.0,
            Self::OrientalInk => 3.0,
            Self::TactileIndustrial => 3.0,
        }
    }

    pub fn card_radius(self) -> f32 {
        match self {
            Self::Win11Mica => 6.0,
            Self::MacosBento => 12.0,
            Self::LinearDark => 6.0,
            Self::OrientalInk => 4.0,
            Self::TactileIndustrial => 4.0,
        }
    }

    pub fn border_width(self) -> f32 {
        match self {
            Self::TactileIndustrial => 1.4,
            _ => 1.0,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Win11Mica,
            1 => Self::MacosBento,
            2 => Self::LinearDark,
            3 => Self::OrientalInk,
            _ => Self::TactileIndustrial,
        }
    }
}

pub fn active_design_style() -> UiDesignStyle {
    UiDesignStyle::from_u8(ACTIVE_DESIGN_STYLE.load(Ordering::Acquire))
}

pub fn set_active_design_style(style: UiDesignStyle) {
    ACTIVE_DESIGN_STYLE.store(style as u8, Ordering::Release);
}

pub fn save_design_style(style: UiDesignStyle) {
    set_active_design_style(style);
    let _ = crate::tool_prefs::set_ui_design_style(style.as_config());
}

#[derive(Clone, Copy)]
pub struct UiPalette {
    pub style: UiDesignStyle,
    pub control_radius: f32,
    pub card_radius: f32,
    pub border_width: f32,
    pub app_bg: Color32,
    pub nav_bg: Color32,
    pub command_bar: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub control_bg: Color32,
    pub control_hover: Color32,
    pub nav_hover: Color32,
    pub nav_selected: Color32,
    pub border: Color32,
    pub border_subtle: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub warning: Color32,
    pub warning_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub success: Color32,
    pub success_soft: Color32,
    pub info_soft: Color32,
    pub row_hover: Color32,
}

impl UiPalette {
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        Self::for_style(active_design_style(), visuals.dark_mode)
    }

    pub fn for_style(style: UiDesignStyle, dark_mode: bool) -> Self {
        let control_radius = style.control_radius();
        let card_radius = style.card_radius();
        let border_width = style.border_width();
        match style {
            UiDesignStyle::Win11Mica => {
                if dark_mode {
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(30, 32, 36),
                        nav_bg: Color32::from_rgb(25, 27, 31),
                        command_bar: Color32::from_rgb(34, 36, 41),
                        surface: Color32::from_rgb(41, 44, 50),
                        surface_alt: Color32::from_rgb(35, 38, 43),
                        control_bg: Color32::from_rgb(48, 52, 58),
                        control_hover: Color32::from_rgb(56, 61, 68),
                        nav_hover: Color32::from_rgb(45, 50, 58),
                        nav_selected: Color32::from_rgb(52, 93, 135),
                        border: Color32::from_rgb(68, 73, 82),
                        border_subtle: Color32::from_rgb(50, 54, 62),
                        text: Color32::from_rgb(244, 246, 249),
                        muted: Color32::from_rgb(186, 194, 205),
                        accent: Color32::from_rgb(96, 180, 255),
                        accent_hover: Color32::from_rgb(135, 199, 255),
                        accent_pressed: Color32::from_rgb(67, 156, 235),
                        accent_soft: Color32::from_rgb(39, 64, 92),
                        accent_text: Color32::WHITE,
                        warning: Color32::from_rgb(255, 202, 109),
                        warning_soft: Color32::from_rgb(66, 52, 30),
                        danger: Color32::from_rgb(255, 112, 112),
                        danger_soft: Color32::from_rgb(66, 34, 34),
                        success: Color32::from_rgb(96, 180, 255),
                        success_soft: Color32::from_rgb(28, 58, 48),
                        info_soft: Color32::from_rgb(38, 48, 58),
                        row_hover: Color32::from_rgb(48, 54, 64),
                    }
                } else {
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(240, 242, 245),
                        nav_bg: Color32::from_rgb(231, 234, 239),
                        command_bar: Color32::from_rgb(238, 240, 244),
                        surface: Color32::from_rgb(250, 251, 252),
                        surface_alt: Color32::from_rgb(236, 240, 245),
                        control_bg: Color32::WHITE,
                        control_hover: Color32::from_rgb(229, 237, 246),
                        nav_hover: Color32::from_rgb(216, 225, 236),
                        nav_selected: Color32::from_rgb(0, 103, 192),
                        border: Color32::from_rgb(172, 180, 190),
                        border_subtle: Color32::from_rgb(202, 209, 218),
                        text: Color32::from_rgb(30, 35, 42),
                        muted: Color32::from_rgb(90, 98, 110),
                        accent: Color32::from_rgb(0, 103, 192),
                        accent_hover: Color32::from_rgb(16, 118, 208),
                        accent_pressed: Color32::from_rgb(0, 84, 158),
                        accent_soft: Color32::from_rgb(222, 236, 250),
                        accent_text: Color32::WHITE,
                        warning: Color32::from_rgb(157, 93, 0),
                        warning_soft: Color32::from_rgb(255, 246, 224),
                        danger: Color32::from_rgb(190, 45, 45),
                        danger_soft: Color32::from_rgb(255, 235, 235),
                        success: Color32::from_rgb(15, 123, 15),
                        success_soft: Color32::from_rgb(226, 246, 234),
                        info_soft: Color32::from_rgb(232, 241, 248),
                        row_hover: Color32::from_rgb(231, 238, 247),
                    }
                }
            }
            UiDesignStyle::MacosBento => {
                if dark_mode {
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(26, 27, 32),
                        nav_bg: Color32::from_rgb(33, 35, 42),
                        command_bar: Color32::from_rgb(31, 33, 39),
                        surface: Color32::from_rgb(40, 43, 52),
                        surface_alt: Color32::from_rgb(47, 50, 60),
                        control_bg: Color32::from_rgb(54, 58, 70),
                        control_hover: Color32::from_rgb(65, 70, 84),
                        nav_hover: Color32::from_rgb(49, 53, 66),
                        nav_selected: Color32::from_rgb(10, 132, 255),
                        border: Color32::from_rgb(76, 81, 96),
                        border_subtle: Color32::from_rgb(57, 61, 74),
                        text: Color32::from_rgb(245, 246, 250),
                        muted: Color32::from_rgb(172, 178, 194),
                        accent: Color32::from_rgb(10, 132, 255),
                        accent_hover: Color32::from_rgb(56, 156, 255),
                        accent_pressed: Color32::from_rgb(0, 110, 224),
                        accent_soft: Color32::from_rgb(32, 68, 112),
                        accent_text: Color32::WHITE,
                        warning: Color32::from_rgb(255, 214, 10),
                        warning_soft: Color32::from_rgb(70, 56, 22),
                        danger: Color32::from_rgb(255, 69, 58),
                        danger_soft: Color32::from_rgb(72, 30, 32),
                        success: Color32::from_rgb(48, 209, 88),
                        success_soft: Color32::from_rgb(26, 64, 42),
                        info_soft: Color32::from_rgb(34, 56, 86),
                        row_hover: Color32::from_rgb(50, 55, 68),
                    }
                } else {
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(242, 243, 247),
                        nav_bg: Color32::from_rgb(233, 235, 241),
                        command_bar: Color32::from_rgb(237, 238, 244),
                        surface: Color32::from_rgb(255, 255, 255),
                        surface_alt: Color32::from_rgb(245, 246, 250),
                        control_bg: Color32::from_rgb(248, 249, 252),
                        control_hover: Color32::from_rgb(232, 240, 254),
                        nav_hover: Color32::from_rgb(220, 226, 238),
                        nav_selected: Color32::from_rgb(0, 122, 255),
                        border: Color32::from_rgb(200, 205, 216),
                        border_subtle: Color32::from_rgb(224, 227, 235),
                        text: Color32::from_rgb(28, 28, 30),
                        muted: Color32::from_rgb(108, 112, 122),
                        accent: Color32::from_rgb(0, 122, 255),
                        accent_hover: Color32::from_rgb(24, 138, 255),
                        accent_pressed: Color32::from_rgb(0, 98, 204),
                        accent_soft: Color32::from_rgb(225, 239, 255),
                        accent_text: Color32::WHITE,
                        warning: Color32::from_rgb(180, 105, 0),
                        warning_soft: Color32::from_rgb(255, 245, 220),
                        danger: Color32::from_rgb(215, 40, 40),
                        danger_soft: Color32::from_rgb(255, 234, 234),
                        success: Color32::from_rgb(30, 150, 65),
                        success_soft: Color32::from_rgb(226, 248, 233),
                        info_soft: Color32::from_rgb(229, 241, 255),
                        row_hover: Color32::from_rgb(236, 242, 252),
                    }
                }
            }
            UiDesignStyle::LinearDark => {
                // Linear / Raycast Pro Utility — always high-contrast obsidian slate
                let _ = dark_mode;
                Self {
                    style,
                    control_radius,
                    card_radius,
                    border_width,
                    app_bg: Color32::from_rgb(13, 15, 19),
                    nav_bg: Color32::from_rgb(10, 12, 16),
                    command_bar: Color32::from_rgb(16, 19, 24),
                    surface: Color32::from_rgb(21, 24, 31),
                    surface_alt: Color32::from_rgb(26, 30, 39),
                    control_bg: Color32::from_rgb(30, 35, 45),
                    control_hover: Color32::from_rgb(39, 46, 60),
                    nav_hover: Color32::from_rgb(28, 33, 44),
                    nav_selected: Color32::from_rgb(94, 106, 210),
                    border: Color32::from_rgb(52, 60, 76),
                    border_subtle: Color32::from_rgb(35, 41, 53),
                    text: Color32::from_rgb(242, 244, 248),
                    muted: Color32::from_rgb(154, 164, 182),
                    accent: Color32::from_rgb(56, 189, 248),
                    accent_hover: Color32::from_rgb(103, 210, 255),
                    accent_pressed: Color32::from_rgb(14, 165, 233),
                    accent_soft: Color32::from_rgb(25, 52, 78),
                    accent_text: Color32::from_rgb(10, 14, 20),
                    warning: Color32::from_rgb(251, 191, 36),
                    warning_soft: Color32::from_rgb(62, 46, 16),
                    danger: Color32::from_rgb(248, 113, 113),
                    danger_soft: Color32::from_rgb(68, 26, 30),
                    success: Color32::from_rgb(52, 211, 153),
                    success_soft: Color32::from_rgb(18, 58, 46),
                    info_soft: Color32::from_rgb(24, 46, 68),
                    row_hover: Color32::from_rgb(29, 34, 45),
                }
            }
            UiDesignStyle::OrientalInk => {
                if dark_mode {
                    // Night ink stone (端砚夜墨)
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(26, 26, 24),
                        nav_bg: Color32::from_rgb(22, 22, 20),
                        command_bar: Color32::from_rgb(30, 30, 28),
                        surface: Color32::from_rgb(36, 36, 33),
                        surface_alt: Color32::from_rgb(43, 42, 38),
                        control_bg: Color32::from_rgb(48, 47, 43),
                        control_hover: Color32::from_rgb(58, 56, 51),
                        nav_hover: Color32::from_rgb(45, 44, 40),
                        nav_selected: Color32::from_rgb(188, 62, 52),
                        border: Color32::from_rgb(82, 79, 72),
                        border_subtle: Color32::from_rgb(58, 56, 50),
                        text: Color32::from_rgb(238, 233, 222),
                        muted: Color32::from_rgb(176, 170, 158),
                        accent: Color32::from_rgb(209, 78, 66),
                        accent_hover: Color32::from_rgb(226, 98, 86),
                        accent_pressed: Color32::from_rgb(178, 56, 46),
                        accent_soft: Color32::from_rgb(76, 38, 34),
                        accent_text: Color32::from_rgb(252, 249, 242),
                        warning: Color32::from_rgb(228, 176, 82),
                        warning_soft: Color32::from_rgb(66, 52, 24),
                        danger: Color32::from_rgb(226, 88, 78),
                        danger_soft: Color32::from_rgb(68, 30, 28),
                        success: Color32::from_rgb(78, 176, 152),
                        success_soft: Color32::from_rgb(26, 56, 50),
                        info_soft: Color32::from_rgb(38, 52, 54),
                        row_hover: Color32::from_rgb(46, 45, 41),
                    }
                } else {
                    // Warm Rice Paper & Cinnabar Seal (宣纸水墨 · 朱砂玉青)
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(246, 242, 233),
                        nav_bg: Color32::from_rgb(238, 233, 221),
                        command_bar: Color32::from_rgb(241, 236, 225),
                        surface: Color32::from_rgb(252, 249, 242),
                        surface_alt: Color32::from_rgb(240, 235, 223),
                        control_bg: Color32::from_rgb(254, 252, 247),
                        control_hover: Color32::from_rgb(236, 228, 214),
                        nav_hover: Color32::from_rgb(227, 220, 206),
                        nav_selected: Color32::from_rgb(176, 48, 40),
                        border: Color32::from_rgb(184, 175, 158),
                        border_subtle: Color32::from_rgb(214, 206, 190),
                        text: Color32::from_rgb(32, 33, 34),
                        muted: Color32::from_rgb(102, 98, 90),
                        accent: Color32::from_rgb(176, 48, 40),
                        accent_hover: Color32::from_rgb(196, 62, 52),
                        accent_pressed: Color32::from_rgb(146, 36, 30),
                        accent_soft: Color32::from_rgb(245, 226, 220),
                        accent_text: Color32::from_rgb(254, 252, 247),
                        warning: Color32::from_rgb(162, 102, 16),
                        warning_soft: Color32::from_rgb(250, 239, 214),
                        danger: Color32::from_rgb(176, 48, 40),
                        danger_soft: Color32::from_rgb(248, 226, 222),
                        success: Color32::from_rgb(43, 122, 107),
                        success_soft: Color32::from_rgb(223, 240, 235),
                        info_soft: Color32::from_rgb(228, 237, 234),
                        row_hover: Color32::from_rgb(242, 236, 224),
                    }
                }
            }
            UiDesignStyle::TactileIndustrial => {
                if dark_mode {
                    // Anodized Gunmetal & Industrial Orange (深空枪灰工程版)
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(28, 30, 31),
                        nav_bg: Color32::from_rgb(22, 24, 25),
                        command_bar: Color32::from_rgb(24, 26, 27),
                        surface: Color32::from_rgb(37, 40, 42),
                        surface_alt: Color32::from_rgb(44, 47, 50),
                        control_bg: Color32::from_rgb(49, 53, 56),
                        control_hover: Color32::from_rgb(62, 66, 70),
                        nav_hover: Color32::from_rgb(42, 45, 48),
                        nav_selected: Color32::from_rgb(255, 85, 0),
                        border: Color32::from_rgb(92, 96, 100),
                        border_subtle: Color32::from_rgb(62, 66, 70),
                        text: Color32::from_rgb(242, 240, 234),
                        muted: Color32::from_rgb(174, 174, 168),
                        accent: Color32::from_rgb(255, 85, 0),
                        accent_hover: Color32::from_rgb(255, 112, 38),
                        accent_pressed: Color32::from_rgb(218, 68, 0),
                        accent_soft: Color32::from_rgb(82, 42, 22),
                        accent_text: Color32::WHITE,
                        warning: Color32::from_rgb(245, 176, 36),
                        warning_soft: Color32::from_rgb(68, 52, 18),
                        danger: Color32::from_rgb(242, 72, 58),
                        danger_soft: Color32::from_rgb(72, 28, 24),
                        success: Color32::from_rgb(46, 194, 118),
                        success_soft: Color32::from_rgb(22, 58, 40),
                        info_soft: Color32::from_rgb(38, 48, 56),
                        row_hover: Color32::from_rgb(48, 52, 55),
                    }
                } else {
                    // Style 5 Default: Teenage Engineering Matte Aluminum & Industrial Orange
                    Self {
                        style,
                        control_radius,
                        card_radius,
                        border_width,
                        app_bg: Color32::from_rgb(232, 229, 221),
                        nav_bg: Color32::from_rgb(221, 217, 208),
                        command_bar: Color32::from_rgb(225, 221, 212),
                        surface: Color32::from_rgb(244, 242, 236),
                        surface_alt: Color32::from_rgb(235, 232, 224),
                        control_bg: Color32::from_rgb(250, 248, 243),
                        control_hover: Color32::from_rgb(255, 237, 226),
                        nav_hover: Color32::from_rgb(208, 204, 194),
                        nav_selected: Color32::from_rgb(255, 85, 0),
                        border: Color32::from_rgb(138, 134, 126),
                        border_subtle: Color32::from_rgb(184, 180, 170),
                        text: Color32::from_rgb(24, 25, 26),
                        muted: Color32::from_rgb(92, 90, 84),
                        accent: Color32::from_rgb(255, 85, 0),
                        accent_hover: Color32::from_rgb(230, 72, 0),
                        accent_pressed: Color32::from_rgb(196, 58, 0),
                        accent_soft: Color32::from_rgb(255, 226, 208),
                        accent_text: Color32::WHITE,
                        warning: Color32::from_rgb(184, 98, 0),
                        warning_soft: Color32::from_rgb(252, 238, 206),
                        danger: Color32::from_rgb(204, 38, 28),
                        danger_soft: Color32::from_rgb(252, 226, 222),
                        success: Color32::from_rgb(24, 132, 74),
                        success_soft: Color32::from_rgb(218, 242, 226),
                        info_soft: Color32::from_rgb(226, 233, 238),
                        row_hover: Color32::from_rgb(238, 233, 222),
                    }
                }
            }
        }
    }
}

pub fn sync_theme_from_prefs(ctx: &egui::Context) -> egui::ThemePreference {
    let style = UiDesignStyle::from_config(&crate::tool_prefs::ui_design_style());
    set_active_design_style(style);
    let pref = if style == UiDesignStyle::LinearDark {
        egui::ThemePreference::Dark
    } else {
        match crate::tool_prefs::ui_theme_preference()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "dark" => egui::ThemePreference::Dark,
            "system" => egui::ThemePreference::System,
            _ => egui::ThemePreference::Light,
        }
    };
    ctx.set_theme(pref);
    pref
}

pub fn save_theme_preference(pref: egui::ThemePreference) {
    let value = match pref {
        egui::ThemePreference::Dark => "dark",
        egui::ThemePreference::System => "system",
        egui::ThemePreference::Light => "light",
    };
    let _ = crate::tool_prefs::set_ui_theme_preference(value);
}

pub fn apply_tool_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let palette = UiPalette::from_visuals(&style.visuals);
    let radius = egui::Rounding::same(palette.control_radius);
    style.spacing.item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
    style.spacing.button_padding = egui::vec2(SPACE_MD, 6.0);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style.visuals.window_rounding = egui::Rounding::same(palette.card_radius);
    style.visuals.menu_rounding = radius;
    style.visuals.window_fill = palette.app_bg;
    style.visuals.panel_fill = palette.app_bg;
    style.visuals.extreme_bg_color = palette.control_bg;
    style.visuals.faint_bg_color = palette.surface_alt;
    style.visuals.widgets.noninteractive.fg_stroke.color = palette.text;
    style.visuals.widgets.inactive.fg_stroke.color = palette.text;
    style.visuals.widgets.hovered.fg_stroke.color = palette.text;
    style.visuals.widgets.active.fg_stroke.color = palette.text;
    style.visuals.widgets.open.fg_stroke.color = palette.text;
    style.visuals.widgets.inactive.bg_fill = palette.control_bg;
    style.visuals.widgets.inactive.weak_bg_fill = palette.control_bg;
    style.visuals.widgets.hovered.bg_fill = palette.control_hover;
    style.visuals.widgets.hovered.weak_bg_fill = palette.control_hover;
    style.visuals.widgets.active.bg_fill = palette.accent_soft;
    style.visuals.widgets.active.weak_bg_fill = palette.accent_soft;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(palette.border_width, palette.border);
    style.visuals.widgets.hovered.bg_stroke =
        Stroke::new(palette.border_width, palette.accent_hover);
    style.visuals.widgets.active.bg_stroke =
        Stroke::new(palette.border_width, palette.accent_pressed);
    style.visuals.widgets.inactive.rounding = radius;
    style.visuals.widgets.hovered.rounding = radius;
    style.visuals.widgets.active.rounding = radius;
    style.visuals.widgets.open.rounding = radius;
    style.visuals.selection.bg_fill = palette.accent_soft;
    style.visuals.selection.stroke = Stroke::new(palette.border_width, palette.accent);
    style.visuals.hyperlink_color = palette.accent;
    ctx.set_style(style);
}

pub fn surface_card(ui: &mut Ui, selected: bool, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let palette = UiPalette::from_visuals(ui.visuals());
    Frame::none()
        .fill(if selected {
            palette.accent_soft
        } else {
            palette.surface
        })
        .stroke(Stroke::new(
            palette.border_width,
            if selected {
                palette.accent
            } else {
                palette.border_subtle
            },
        ))
        .rounding(palette.card_radius)
        .inner_margin(Margin::symmetric(SPACE_MD, 10.0))
        .show(ui, add)
        .response
}

pub fn status_pill(ui: &mut Ui, text: &str, danger: bool) {
    let palette = UiPalette::from_visuals(ui.visuals());
    let (fill, color) = if danger {
        (palette.danger_soft, palette.danger)
    } else {
        (palette.success_soft, palette.success)
    };
    Frame::none()
        .fill(fill)
        .rounding(FLOATING_RADIUS)
        .inner_margin(Margin::symmetric(10.0, SPACE_XS))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}

pub fn empty_state(ui: &mut Ui, symbol: &str, title: &str, hint: &str) {
    let palette = UiPalette::from_visuals(ui.visuals());
    ui.vertical_centered(|ui| {
        ui.add_space(SPACE_XL * 2.0);
        ui.label(RichText::new(symbol).size(32.0).color(palette.accent));
        ui.add_space(SPACE_SM);
        ui.label(RichText::new(title).strong().size(17.0).color(palette.text));
        ui.add_space(SPACE_XS);
        ui.label(RichText::new(hint).color(palette.muted));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_design_style_is_tactile_industrial() {
        assert_eq!(UiDesignStyle::default(), UiDesignStyle::TactileIndustrial);
        assert_eq!(
            UiDesignStyle::from_config(""),
            UiDesignStyle::TactileIndustrial
        );
        assert_eq!(
            UiDesignStyle::from_config("tactile"),
            UiDesignStyle::TactileIndustrial
        );
    }

    #[test]
    fn all_five_design_styles_roundtrip_and_produce_distinct_palettes() {
        for style in UiDesignStyle::ALL {
            assert_eq!(UiDesignStyle::from_config(style.as_config()), style);
            let light = UiPalette::for_style(style, false);
            let dark = UiPalette::for_style(style, true);
            assert_eq!(light.style, style);
            assert_eq!(dark.style, style);
        }
        let s1 = UiPalette::for_style(UiDesignStyle::Win11Mica, false);
        let s2 = UiPalette::for_style(UiDesignStyle::MacosBento, false);
        let s3 = UiPalette::for_style(UiDesignStyle::LinearDark, false);
        let s4 = UiPalette::for_style(UiDesignStyle::OrientalInk, false);
        let s5 = UiPalette::for_style(UiDesignStyle::TactileIndustrial, false);
        assert_ne!(s1.app_bg, s5.app_bg);
        assert_ne!(s2.card_radius, s5.card_radius);
        assert_ne!(s3.app_bg, s4.app_bg);
        assert_ne!(s4.accent, s5.accent);
    }
}
