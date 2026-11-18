//! 各环节耗时日志：只在 debug 构建输出到控制台，release 下整段被编译器去掉

use std::time::Duration;

/// 打一行耗时日志（前缀“[耗时]”，便于在控制台里筛选）
macro_rules! tlog {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            println!("[耗时] {}", format_args!($($arg)*));
        }
    };
}

/// 毫秒，保留一位小数
pub fn ms(d: Duration) -> String {
    format!("{:.1}ms", d.as_secs_f64() * 1000.0)
}
