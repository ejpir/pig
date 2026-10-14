use anyhow::{Context, Result, bail};
use pi_quickjs_backend_poc::{Options, fixture, spawn};
use std::{path::PathBuf, time::Duration};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--fixture"] {
        let result = fixture::demo(|event| println!("{event}"))?;
        eprintln!(
            "PASS: upstream Pi + OpenAI SDK in QuickJS; streamed read/write/read cycle; {} transcript messages",
            result["messages"].as_array().unwrap().len()
        );
        return Ok(());
    }
    if args.is_empty() || args == ["--help"] {
        println!(
            "pi-quickjs-backend-poc --fixture\n\nLive (may call a paid provider and overwrite workspace files):\n  PI_QUICKJS_API_KEY=... pi-quickjs-backend-poc --live WORKSPACE BASE_URL MODEL PROMPT\n\nBASE_URL must use HTTPS, e.g. https://api.openai.com/v1.\nNo shell, durable sessions, Android UI or background service in this PoC."
        );
        return Ok(());
    }
    if args.len() != 5 || args[0] != "--live" {
        bail!("invalid arguments; use --help");
    }
    let result = spawn(
        Options {
            workspace: PathBuf::from(&args[1]),
            base_url: args[2].clone(),
            model: args[3].clone(),
            prompt: args[4].clone(),
            api_key: std::env::var("PI_QUICKJS_API_KEY")
                .context("set PI_QUICKJS_API_KEY for an explicit live run")?,
            allow_loopback_http: false,
            http_chunk_bytes: 4096,
            request_timeout_ms: 30_000,
            timeout: Duration::from_secs(90),
        },
        |event| println!("{event}"),
    )
    .join()?;
    eprintln!(
        "Completed: {} transcript messages",
        result["messages"].as_array().unwrap().len()
    );
    Ok(())
}
