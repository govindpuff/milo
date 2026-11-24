use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let browser = browser::Browser::connect(Duration::from_secs(40)).await?;
    for tab in browser.tabs().await? {
        println!("{} | {}", tab.title, tab.url);
    }
    match std::env::args().nth(1).as_deref() {
        Some("pause") => println!("paused: {:?}", browser.media(false).await),
        Some("play") => println!("resumed: {:?}", browser.media(true).await),
        Some("look") => {
            for tab in browser.tabs().await? {
                let seen = browser.look(&tab.id).await?;
                println!("{}", seen.text.lines().filter(|line| line.contains("video")).collect::<Vec<_>>().join("\n"));
            }
        }
        Some("cycle") => {
            for tab in browser.tabs().await? {
                browser.peek(&tab.id, "document.querySelectorAll('video').forEach(m => m.play())").await;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            println!("paused: {:?}", browser.media(false).await);
            println!("paused again: {:?}", browser.media(false).await);
            println!("resumed: {:?}", browser.media(true).await);
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("paused: {:?}", browser.media(false).await);
        }
        Some("state") => {
            for tab in browser.tabs().await? {
                let script = "JSON.stringify([...document.querySelectorAll('video,audio')].map(m => [m.paused, m.ended, Math.round(m.currentTime)]))";
                println!("state: {:?}", browser.peek(&tab.id, script).await);
            }
        }
        _ => {}
    }
    Ok(())
}
