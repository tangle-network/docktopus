use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::time::Duration;

#[cfg(feature = "deploy")]
use reqwest::Client;

#[cfg(feature = "deploy")]
use tokio::time::sleep;

#[cfg(feature = "deploy")]
#[derive(Debug, thiserror::Error)]
pub enum HealthCheckError {
    #[error("Health check failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Health check failed: expected status {expected}, got {actual}")]
    UnexpectedStatus { expected: u16, actual: u16 },
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum Method {
    Get,
    Post,
}

impl Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Method::Get => write!(f, "GET"),
            Method::Post => write!(f, "POST"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub endpoint: String,
    pub method: Method,
    pub expected_status: u16,
    pub body: Option<String>,
    #[serde(with = "duration_serde")]
    pub interval: Duration,
    #[serde(with = "duration_serde")]
    pub timeout: Duration,
    pub retries: u32,
}

// Custom serialization for Duration
pub(crate) mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_nanos().serialize(serializer)
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let nanos = u128::deserialize(deserializer)?;
        let secs = nanos / 1_000_000_000;
        let nanos = (nanos % 1_000_000_000) as u32;
        Ok(Duration::new(secs as u64, nanos))
    }
}

#[cfg(feature = "deploy")]
impl HealthCheck {
    /// Perform the health check
    ///
    /// # Errors
    ///
    /// Returns an error if, after the specified number of retries, the health check continues to fail.
    pub async fn check(&self) -> Result<(), HealthCheckError> {
        let client = Client::new();
        let mut attempts = 0;

        while attempts < self.retries {
            match self.perform_check(&client).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts == self.retries {
                        return Err(e);
                    }
                    sleep(self.interval).await;
                }
            }
        }

        Ok(())
    }

    async fn perform_check(&self, client: &Client) -> Result<(), HealthCheckError> {
        let mut request = match self.method {
            Method::Get => client.get(&self.endpoint),
            Method::Post => client.post(&self.endpoint),
        };

        if let Some(body) = &self.body {
            request = request.body(body.clone());
        }

        let response = request
            .timeout(self.timeout)
            .send()
            .await
            .map_err(HealthCheckError::Request)?;

        let status = response.status().as_u16();
        if status != self.expected_status {
            return Err(HealthCheckError::UnexpectedStatus {
                expected: self.expected_status,
                actual: status,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    async fn serve_statuses(statuses: &'static [u16]) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = format!("http://{}/health", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            timeout(Duration::from_secs(5), async move {
                for status in statuses {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut stream = BufReader::new(stream);
                    let mut line = String::new();
                    stream.read_line(&mut line).await.unwrap();
                    assert_eq!(line, "GET /health HTTP/1.1\r\n");

                    loop {
                        line.clear();
                        assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                        if line == "\r\n" {
                            break;
                        }
                    }

                    let response = format!(
                        "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    stream
                        .get_mut()
                        .write_all(response.as_bytes())
                        .await
                        .unwrap();
                    stream.get_mut().shutdown().await.unwrap();
                }
            })
            .await
            .expect("health check did not send all expected requests");
        });
        (endpoint, server)
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let (endpoint, server) = serve_statuses(&[200]).await;
        let health_check = HealthCheck {
            endpoint,
            method: Method::Get,
            expected_status: 200,
            body: None,
            interval: Duration::from_millis(10),
            timeout: Duration::from_secs(1),
            retries: 3,
        };

        let result = health_check.check().await;
        assert!(result.is_ok(), "{result:?}");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_health_check_failure() {
        let (endpoint, server) = serve_statuses(&[500, 500]).await;
        let health_check = HealthCheck {
            endpoint,
            method: Method::Get,
            expected_status: 200,
            body: None,
            interval: Duration::from_millis(10),
            timeout: Duration::from_secs(1),
            retries: 2,
        };

        let result = health_check.check().await;
        assert!(
            matches!(
                result,
                Err(HealthCheckError::UnexpectedStatus {
                    expected: 200,
                    actual: 500
                })
            ),
            "{result:?}"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_health_check_recovers_after_retry() {
        let (endpoint, server) = serve_statuses(&[503, 200]).await;
        let health_check = HealthCheck {
            endpoint,
            method: Method::Get,
            expected_status: 200,
            body: None,
            interval: Duration::from_millis(10),
            timeout: Duration::from_secs(1),
            retries: 3,
        };

        let result = health_check.check().await;
        assert!(result.is_ok(), "{result:?}");
        server.await.unwrap();
    }
}
