use std::io::BufRead;

use serde_json::Value;

fn main() {
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(wait) = line.strip_prefix("wait ") {
            std::thread::sleep(std::time::Duration::from_millis(wait.trim().parse().unwrap_or(0)));
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            println!("not JSON: {line}");
            continue;
        };
        match control::call(&request) {
            Ok(mut answer) => {
                if let Some(image) = answer.get_mut("image") {
                    *image = Value::from(format!("{} bytes", image.as_str().map_or(0, str::len)));
                }
                println!("{}", answer["text"].as_str().unwrap_or_default());
                println!("image: {}", answer["image"]);
            }
            Err(text) => println!("FAILED: {text}"),
        }
        println!("----");
    }
}
