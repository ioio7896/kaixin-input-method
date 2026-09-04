use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Serialize)]
struct PerfMetric<'a> {
    schema_version: u32,
    metric: &'a str,
    samples: usize,
    average_us: f64,
    p50_us: u128,
    p95_us: u128,
    p99_us: u128,
    max_us: u128,
}

fn percentile(sorted: &[Duration], pct: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

pub(super) fn duration_us(value: Duration) -> u128 {
    value.as_micros()
}

pub(super) fn print_stats(
    json_path: Option<&Path>,
    label: &str,
    samples: &[Duration],
) -> (u128, u128) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let total_us: u128 = sorted.iter().map(|duration| duration.as_micros()).sum();
    let average_us = total_us as f64 / sorted.len().max(1) as f64;
    let p50_us = duration_us(percentile(&sorted, 0.50));
    let p95_us = duration_us(percentile(&sorted, 0.95));
    let p99_us = duration_us(percentile(&sorted, 0.99));
    let max_us = duration_us(sorted.last().copied().unwrap_or_default());
    println!(
        "lookup_us_{label} samples={} avg={average_us:.1} p50={p50_us} p95={p95_us} p99={p99_us} max={max_us}",
        sorted.len()
    );

    if let Some(path) = json_path {
        let metric = PerfMetric {
            schema_version: 1,
            metric: label,
            samples: sorted.len(),
            average_us,
            p50_us,
            p95_us,
            p99_us,
            max_us,
        };
        let mut output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|err| panic!("open performance JSON {}: {err}", path.display()));
        serde_json::to_writer(&mut output, &metric).expect("serialize performance metric");
        writeln!(output).expect("finish performance metric line");
    }

    (p95_us, p99_us)
}
