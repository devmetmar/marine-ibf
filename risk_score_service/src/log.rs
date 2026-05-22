//! Timestamped stderr logging: `YYYY-MM-DD HH:MM:SS\t<message>`.

use chrono::Local;

fn timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Satu baris log ke stderr: waktu, tab, pesan.
pub fn log_msg(msg: impl AsRef<str>) {
    eprintln!("{}\t{}", timestamp(), msg.as_ref());
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => {
        $crate::log::log_msg(format!($($t)*))
    };
}
