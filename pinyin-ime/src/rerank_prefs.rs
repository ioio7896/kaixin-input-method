//! 从用户配置 INI 读取候选重排参数（与设置程序写入一致）。
//! 目标：运行中热加载（无需重启输入服务），同时避免每次按键都读文件。

#[derive(Clone, Copy, Debug)]
pub struct RerankPrefs {
    /// 单字通道权重（默认 0.7）
    pub w_single_lm: f64,
    /// 词组路径通道权重（默认 0.2）
    pub w_phrase_path: f64,
    /// 单字通道缩放（默认 2.8）
    pub lm_single_scale: f64,
}

impl Default for RerankPrefs {
    fn default() -> Self {
        Self {
            w_single_lm: 0.82,
            w_phrase_path: 0.16,
            lm_single_scale: 3.35,
        }
    }
}

pub(crate) fn parse_rank_section(text: &str) -> RerankPrefs {
    let mut out = RerankPrefs::default();
    let mut in_rank = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let name = line[1..line.len() - 1].trim().to_ascii_lowercase();
            in_rank = name == "rank";
            continue;
        }
        if !in_rank {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        let parsed = val.parse::<f64>().ok().filter(|x| x.is_finite());
        match key.as_str() {
            "w_single_lm" => {
                if let Some(x) = parsed {
                    out.w_single_lm = x.clamp(0.0, 1.0);
                }
            }
            "w_phrase_path" => {
                if let Some(x) = parsed {
                    out.w_phrase_path = x.clamp(0.0, 1.0);
                }
            }
            "lm_single_scale" => {
                if let Some(x) = parsed {
                    out.lm_single_scale = x.clamp(0.1, 30.0);
                }
            }
            _ => {}
        }
    }
    out
}

/// 获取当前重排偏好；最多每 800ms 检查一次文件时间戳，避免热路径频繁 IO。
pub fn get_rerank_prefs() -> RerankPrefs {
    crate::runtime_config::snapshot().rerank
}
