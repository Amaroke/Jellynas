use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const KEY: &str = "dXBzLXNodXRkb3duLWtleQ==";

#[derive(Debug)]
struct SelfSigned(Arc<CryptoProvider>);

impl ServerCertVerifier for SelfSigned {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            signed,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            signed,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn mask() -> [u8; 4] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    nanos.to_le_bytes()
}

pub fn frame(text: &str, mask: [u8; 4]) -> Vec<u8> {
    let data = text.as_bytes();
    let mut found = vec![0x81];
    match data.len() {
        size if size < 126 => found.push(0x80 | size as u8),
        size if size < 65536 => {
            found.push(0x80 | 126);
            found.extend((size as u16).to_be_bytes());
        }
        size => {
            found.push(0x80 | 127);
            found.extend((size as u64).to_be_bytes());
        }
    }
    found.extend(mask);
    found.extend(
        data.iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    found
}

fn exact(stream: &mut impl Read, size: usize) -> Result<Vec<u8>, String> {
    let mut data = vec![0; size];
    stream
        .read_exact(&mut data)
        .map_err(|error| format!("TrueNAS closed the websocket: {error}"))?;
    Ok(data)
}

pub fn receive(stream: &mut impl Read) -> Result<String, String> {
    let mut message = vec![];
    loop {
        let head = exact(stream, 2)?;
        let size = match head[1] & 0x7F {
            126 => u64::from(u16::from_be_bytes(exact_pair(stream)?)),
            127 => u64::from_be_bytes(exact(stream, 8)?.try_into().expect("eight bytes")),
            size => u64::from(size),
        };
        let mask = if head[1] & 0x80 != 0 {
            Some(exact(stream, 4)?)
        } else {
            None
        };
        let mut data = exact(stream, size as usize)?;
        if let Some(mask) = mask {
            data.iter_mut()
                .enumerate()
                .for_each(|(index, byte)| *byte ^= mask[index % 4]);
        }
        match head[0] & 0x0F {
            0x8 => return Err("TrueNAS closed the websocket".to_string()),
            0x9 | 0xA => continue,
            _ => message.extend(data),
        }
        if head[0] & 0x80 != 0 {
            return String::from_utf8(message).map_err(|error| error.to_string());
        }
    }
}

fn exact_pair(stream: &mut impl Read) -> Result<[u8; 2], String> {
    let data = exact(stream, 2)?;
    Ok([data[0], data[1]])
}

pub struct Socket<S: Read + Write> {
    stream: S,
    calls: u64,
}

impl<S: Read + Write> Socket<S> {
    pub fn new(stream: S) -> Self {
        Socket { stream, calls: 0 }
    }

    pub fn upgrade(mut stream: S, host: &str, path: &str) -> Result<Self, String> {
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("cannot reach TrueNAS: {error}"))?;
        let mut head = vec![];
        while !head.ends_with(b"\r\n\r\n") {
            head.extend(exact(&mut stream, 1)?);
            if head.len() > 16384 {
                return Err("TrueNAS sent no end to its headers".to_string());
            }
        }
        let line = String::from_utf8_lossy(&head);
        let line = line.lines().next().unwrap_or_default();
        if line.split_whitespace().nth(1) != Some("101") {
            return Err(format!("TrueNAS answered {line} to the websocket"));
        }
        Ok(Socket::new(stream))
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.calls += 1;
        let asked = json!({"jsonrpc": "2.0", "id": self.calls, "method": method, "params": params});
        self.stream
            .write_all(&frame(&asked.to_string(), mask()))
            .map_err(|error| format!("cannot write to TrueNAS: {error}"))?;
        loop {
            let answer: Value = serde_json::from_str(&receive(&mut self.stream)?)
                .map_err(|error| error.to_string())?;
            if answer["id"] == json!(self.calls) {
                if let Some(error) = answer.get("error") {
                    return Err(format!("{method} failed: {error}"));
                }
                return Ok(answer["result"].clone());
            }
        }
    }
}

pub fn connect(host: &str) -> Result<Socket<StreamOwned<ClientConnection, TcpStream>>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SelfSigned(provider)))
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string()).map_err(|error| error.to_string())?;
    let tcp = TcpStream::connect((host, 443))
        .map_err(|error| format!("cannot reach TrueNAS: {error}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    let tls = ClientConnection::new(Arc::new(config), name).map_err(|error| error.to_string())?;
    Socket::upgrade(StreamOwned::new(tls, tcp), host, "/api/current")
}

pub fn shutdown(host: &str, key: &str, reason: &str) -> Result<(), String> {
    let mut socket = connect(host)?;
    if socket.call("auth.login_with_api_key", json!([key]))? != json!(true) {
        return Err("TrueNAS refused the API key".to_string());
    }
    socket.call("system.shutdown", json!([reason]))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Fake {
        incoming: Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }

    impl Read for Fake {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.incoming.read(buffer)
        }
    }

    impl Write for Fake {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.sent.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn server(text: &str) -> Vec<u8> {
        let mut found = vec![0x81];
        if text.len() < 126 {
            found.push(text.len() as u8);
        } else {
            found.push(126);
            found.extend((text.len() as u16).to_be_bytes());
        }
        found.extend(text.as_bytes());
        found
    }

    #[test]
    fn a_frame_is_masked_text() {
        let sent = frame("Hi", [1, 2, 3, 4]);
        assert_eq!(vec![0x81, 0x82, 1, 2, 3, 4, b'H' ^ 1, b'i' ^ 2], sent);
        let long = frame(&"x".repeat(300), [0; 4]);
        assert_eq!([0x81, 0x80 | 126, 1, 44], long[..4]);
    }

    #[test]
    fn a_message_skips_pings_and_joins_its_fragments() {
        let mut incoming = vec![0x89, 0x00, 0x01, 0x02];
        incoming.extend(b"ab");
        incoming.extend([0x80, 0x01, b'c']);
        let long = "y".repeat(200);
        incoming.extend(server(&long));
        let mut stream = Cursor::new(incoming);
        assert_eq!("abc", receive(&mut stream).unwrap());
        assert_eq!(long, receive(&mut stream).unwrap());
        assert!(receive(&mut Cursor::new(vec![0x88, 0x00])).is_err());
    }

    #[test]
    fn a_call_waits_for_its_own_answer() {
        let mut incoming =
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n".to_vec();
        incoming.extend(server(
            r#"{"jsonrpc":"2.0","method":"collection_update","params":{}}"#,
        ));
        incoming.extend(server(r#"{"jsonrpc":"2.0","id":1,"result":true}"#));
        incoming.extend(server(
            r#"{"jsonrpc":"2.0","id":2,"error":{"message":"denied"}}"#,
        ));
        let fake = Fake {
            incoming: Cursor::new(incoming),
            sent: vec![],
        };
        let mut socket = Socket::upgrade(fake, "nas", "/api/current").unwrap();
        assert_eq!(
            json!(true),
            socket
                .call("auth.login_with_api_key", json!(["k"]))
                .unwrap()
        );
        let error = socket.call("system.shutdown", json!(["low"])).unwrap_err();
        assert!(error.contains("system.shutdown failed"));
        let sent = String::from_utf8_lossy(&socket.stream.sent).to_string();
        assert!(sent.starts_with("GET /api/current HTTP/1.1\r\nHost: nas\r\n"));
    }

    #[test]
    fn a_refused_upgrade_is_an_error() {
        let fake = Fake {
            incoming: Cursor::new(b"HTTP/1.1 401 Unauthorized\r\n\r\n".to_vec()),
            sent: vec![],
        };
        let error = Socket::upgrade(fake, "nas", "/api/current").err().unwrap();
        assert!(error.contains("401"));
    }
}
