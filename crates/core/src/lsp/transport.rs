use anyhow::{bail, Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub struct LspWriter<W> {
    inner: W,
}

impl<W: AsyncWriteExt + Unpin> LspWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { inner: writer }
    }

    pub async fn send(&mut self, message: &Value) -> Result<()> {
        let body = serde_json::to_string(message)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.inner.write_all(header.as_bytes()).await?;
        self.inner.write_all(body.as_bytes()).await?;
        self.inner.flush().await?;
        Ok(())
    }
}

pub struct LspReader<R> {
    inner: BufReader<R>,
}

impl<R: AsyncReadExt + Unpin> LspReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            inner: BufReader::new(reader),
        }
    }

    pub async fn recv(&mut self) -> Result<Value> {
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            let read = self.inner.read_line(&mut line).await?;
            if read == 0 {
                bail!("LSP stream closed");
            }

            let line = line.trim();
            if line.is_empty() {
                break;
            }

            if let Some(value) = line.strip_prefix("Content-Length: ") {
                content_length = Some(value.parse().context("invalid Content-Length value")?);
            }
        }

        let length = content_length.context("missing Content-Length header")?;
        let mut buf = vec![0u8; length];
        self.inner.read_exact(&mut buf).await?;
        Ok(serde_json::from_slice(&buf)?)
    }
}
