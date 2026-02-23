// Extraction progress display

use std::io::Write;
use std::time::Duration;

// Display extraction progress
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

// Display extraction completion message
pub fn display_completion(elapsed: Duration) {
    println!("\nelapsed {:.2}s", elapsed.as_secs_f64());
}
