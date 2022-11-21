use std::{
    fmt::Write,
    fs,
    io::Error,
    ops::DerefMut,
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use clap::Parser;
use futures::lock::Mutex;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, AttachParams},
    config::{KubeConfigOptions, Kubeconfig},
    Client, Config,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
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
    src: String,

    #[arg(short, long)]
    dst: String,
}

struct FileProcessReader {
    file: tokio::fs::File,
    cur: u64,
    total: u64,
    pb: Option<Arc<ProgressBar>>,
}

impl FileProcessReader {
    async fn new(file_path: &str) -> FileProcessReader {
        FileProcessReader {
            file: tokio::fs::File::open(file_path).await.unwrap(),
            cur: 0,
            total: tokio::fs::metadata(file_path).await.unwrap().len(),
            pb: None,
        }
    }
}

impl AsyncRead for FileProcessReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let ret = Pin::new(&mut self.file).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = ret {
            self.cur += buf.filled().len() as u64;
            if let Some(pb) = self.pb.as_ref() {
                pb.set_position(self.cur)
            }
        }
        ret
    }
}

#[derive(Debug)]
struct StringWriter {
    str: String,
}

impl AsyncWrite for StringWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        self.str
            .push_str(std::str::from_utf8(buf).unwrap_or("[not utf8]"));
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // src file
    let mut f_reader = FileProcessReader::new(args.src.as_str()).await;

    // process bar
    let pb = Arc::new(ProgressBar::new(f_reader.total));
    pb.set_style(ProgressStyle::with_template(
        "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({binary_bytes_per_sec}, {eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));
    f_reader.pb = Some(pb.clone());

    // kube client
    tracing_subscriber::fmt::init();
    let client = Client::try_from(
        Config::from_custom_kubeconfig(
            Kubeconfig::from_yaml(fs::read_to_string(&args.kubeconfig)?.as_str())?,
            &KubeConfigOptions {
                context: None,
                cluster: None,
                user: None,
            },
        )
        .await?,
    )?;

    // pod exec
    let pods: Api<Pod> = Api::namespaced(client, args.namespace.as_str());
    let mut ap = AttachParams::default().stdin(true);
    if !args.container.is_empty() {
        ap = ap.container(args.container);
    }

    let exec = format!(
        "mkdir -p {} && cd {} && cat > {}",
        args.dst,
        args.dst,
        Path::new(&args.src).file_name().unwrap().to_str().unwrap()
    );

    let mut attached = pods
        .exec(args.pod.as_str(), vec!["sh", "-c", exec.as_str()], &ap)
        .await?;

    // The received streams from `AttachedProcess`
    let mut stdin_writer = attached.stdin().unwrap();
    let mut stdout_reader = attached.stdout().unwrap();
    let mut stderr_reader = attached.stderr().unwrap();

    // stdin
    tokio::spawn(async move {
        tokio::io::copy(&mut f_reader, &mut stdin_writer)
            .await
            .unwrap();
    });

    // stdout
    let stdout = Arc::new(Mutex::new(StringWriter { str: String::new() }));
    let out = stdout.clone();
    tokio::spawn(async move {
        tokio::io::copy(&mut stdout_reader, out.lock().await.deref_mut())
            .await
            .unwrap();
    });

    // stderr
    let stderr = Arc::new(Mutex::new(StringWriter { str: String::new() }));
    let err = stderr.clone();
    tokio::spawn(async move {
        tokio::io::copy(&mut stderr_reader, err.lock().await.deref_mut())
            .await
            .unwrap();
    });

    attached.take_status().unwrap().await;
    pb.abandon();

    if !stdout.lock().await.str.is_empty() {
        info!("stdout:{}", stdout.lock().await.str);
    }
    if !stderr.lock().await.str.is_empty() {
        info!("stderr:{}", stderr.lock().await.str);
    }

    Ok(())
}
