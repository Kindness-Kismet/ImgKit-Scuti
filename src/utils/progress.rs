// 解包进度显示

use std::io::Write;
use std::time::Duration;

// 显示解包进度
pub fn display_progress(filename: &str, current: usize, total: usize) {
    if current.is_multiple_of(10) || current == total {
        let percent = (current as f64 / total as f64) * 100.0;
        print!(
            "\r{} extracting... {:.1}% [{}/{}]",
            filename, percent, current, total
        );
        let _ = std::io::stdout().flush();
    }
}

// 显示解包完成信息
pub fn display_completion(elapsed: Duration) {
    println!("\nelapsed {:.2}s", elapsed.as_secs_f64());
}
