use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UdpSocket;

#[derive(Debug)]
pub struct UdpReader(UdpSocket);

impl UdpReader {
    pub fn new(s: UdpSocket) -> io::Result<Self> {
        use crate::utils::Shutdown;

        crate::utils::shutdown(&s, Shutdown::Write)?;
        Ok(Self(s))
    }
}

impl From<UdpSocket> for UdpReader {
    fn from(u: UdpSocket) -> Self {
        Self(u)
    }
}

impl Deref for UdpReader {
    type Target = UdpSocket;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for UdpReader {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsyncRead for UdpReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.0.poll_recv(cx, buf)
    }
}

#[derive(Debug)]
pub struct UdpWriter(UdpSocket);

impl UdpWriter {
    pub fn new(s: UdpSocket) -> io::Result<Self> {
        use crate::utils::Shutdown;

        crate::utils::shutdown(&s, Shutdown::Read)?;
        Ok(Self(s))
    }
}

impl From<UdpSocket> for UdpWriter {
    fn from(u: UdpSocket) -> Self {
        Self(u)
    }
}

impl Deref for UdpWriter {
    type Target = UdpSocket;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for UdpWriter {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsyncWrite for UdpWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        self.0.poll_send(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
}
