use std::time::Duration;
use std::{cmp::min, fmt::Write};
use std::{fs, thread};

use clap::Parser;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, AttachParams},
    config::{KubeConfigOptions, Kubeconfig},
    Client, Config,
};
use tracing::*;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    kubeconfig: String,

    #[arg(short, long, default_value = "default")]
    namespace: String,

    #[arg(short, long)]
    pod: String,

    #[arg(short, long, default_value = "")]
    container: String,

    #[arg(short, long)]
    tty: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let kubeconfig = fs::read_to_string(args.kubeconfig)?;

    tracing_subscriber::fmt::init();
    // let client = Client::try_default().await?;
    let client = Client::try_from(
        Config::from_custom_kubeconfig(
            Kubeconfig::from_yaml(kubeconfig.as_str())?,
            &KubeConfigOptions {
                context: None,
                cluster: None,
                user: None,
            },
        )
        .await?,
    )?;

    let pods: Api<Pod> = Api::namespaced(client, args.namespace.as_str());
    // Stop on error including a pod already exists or is still being deleted.
    // pods.create(&PostParams::default(), &p).await?;

    // Do an interactive exec to a blog pod with the `sh` command
    let mut ap = AttachParams::interactive_tty().tty(args.tty);
    if args.container.len() > 0 {
        ap = ap.container(args.container);
    }
    let mut attached = pods.exec(args.pod.as_str(), vec!["bash"], &ap).await?;

    // The received streams from `AttachedProcess`
    let mut stdin_writer = attached.stdin().unwrap();
    let mut stdout_reader = attached.stdout().unwrap();

    // > For interactive uses, it is recommended to spawn a thread dedicated to user input and use blocking IO directly in that thread.
    // > https://docs.rs/tokio/0.2.24/tokio/io/fn.stdin.html
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    // pipe current stdin to the stdin writer from ws
    tokio::spawn(async move {
        tokio::io::copy(&mut stdin, &mut stdin_writer)
            .await
            .unwrap();
    });
    // pipe stdout from ws to current stdout
    tokio::spawn(async move {
        tokio::io::copy(&mut stdout_reader, &mut stdout)
            .await
            .unwrap();
    });
    // When done, type `exit\n` to end it, so the pod is deleted.
    let status = attached.take_status().unwrap().await;
    info!("{}", status.unwrap().status.unwrap());

    Ok(())
}
