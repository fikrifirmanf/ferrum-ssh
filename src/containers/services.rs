//! Service hints and ephemeral connection settings. Never persisted or logged.
use super::ports::OpenAs;
use super::{Item, LIST_TIMEOUT, Runtime, SshClient, shell_quote, sudo_prefix};
use anyhow::{Result, anyhow, bail};
use std::{collections::BTreeMap, net::SocketAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Postgres,
    Mysql,
    Mongo,
    Redis,
    ClickHouse,
    ClickHouseHttp,
    Amqp,
    S3,
    Http,
    Https,
    Tcp,
}
impl Service {
    pub const ALL: [Self; 11] = [
        Self::Postgres,
        Self::Mysql,
        Self::Mongo,
        Self::Redis,
        Self::ClickHouse,
        Self::ClickHouseHttp,
        Self::Amqp,
        Self::S3,
        Self::Http,
        Self::Https,
        Self::Tcp,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Mysql => "MySQL / MariaDB",
            Self::Mongo => "MongoDB",
            Self::Redis => "Redis / Valkey",
            Self::ClickHouse => "ClickHouse native",
            Self::ClickHouseHttp => "ClickHouse HTTP",
            Self::Amqp => "RabbitMQ / AMQP",
            Self::S3 => "S3 endpoint",
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::Tcp => "TCP",
        }
    }
    pub fn mode(self) -> OpenAs {
        match self {
            Self::Http | Self::S3 | Self::ClickHouseHttp => OpenAs::Http,
            Self::Https => OpenAs::Https,
            _ => OpenAs::Tcp,
        }
    }
    pub fn has_credentials(self) -> bool {
        !matches!(self, Self::Tcp)
    }
    pub fn env_keys(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => &["POSTGRES_USER", "POSTGRES_PASSWORD", "POSTGRES_DB"],
            Self::Mysql => &[
                "MYSQL_USER",
                "MYSQL_PASSWORD",
                "MYSQL_ROOT_PASSWORD",
                "MYSQL_DATABASE",
                "MARIADB_USER",
                "MARIADB_PASSWORD",
                "MARIADB_ROOT_PASSWORD",
                "MARIADB_DATABASE",
            ],
            Self::Mongo => &[
                "MONGO_INITDB_ROOT_USERNAME",
                "MONGO_INITDB_ROOT_PASSWORD",
                "MONGO_INITDB_DATABASE",
            ],
            Self::Redis => &["REDIS_PASSWORD", "VALKEY_PASSWORD"],
            Self::ClickHouse | Self::ClickHouseHttp => {
                &["CLICKHOUSE_USER", "CLICKHOUSE_PASSWORD", "CLICKHOUSE_DB"]
            }
            Self::Amqp => &[
                "RABBITMQ_DEFAULT_USER",
                "RABBITMQ_DEFAULT_PASS",
                "RABBITMQ_DEFAULT_VHOST",
            ],
            Self::S3 => &["MINIO_ROOT_USER", "MINIO_ROOT_PASSWORD"],
            _ => &[],
        }
    }
}

/// Image basename, not arbitrary container names or host ports, determines the hint.
/// Unrecognized images remain generic. Every hint can be overridden by the user.
pub fn detect(image: &str, port: u16) -> Service {
    let image = image.split('@').next().unwrap_or(image);
    let name = image
        .rsplit('/')
        .next()
        .unwrap_or(image)
        .split(':')
        .next()
        .unwrap_or("");
    match name {
        "postgres" | "postgresql" => Service::Postgres,
        "mysql" | "mariadb" => Service::Mysql,
        "mongo" | "mongodb" | "mongodb-community-server" => Service::Mongo,
        "redis" | "valkey" => Service::Redis,
        "clickhouse" | "clickhouse-server" if matches!(port, 9000 | 9440) => Service::ClickHouse,
        "clickhouse" | "clickhouse-server" if matches!(port, 8123 | 8443) => {
            Service::ClickHouseHttp
        }
        "rabbitmq" if matches!(port, 5672 | 5671) => Service::Amqp,
        "minio" if port == 9000 => Service::S3,
        _ if matches!(port, 443 | 8443) => Service::Https,
        _ if matches!(port, 80 | 8080 | 3000 | 8000 | 8123 | 15672 | 9001) => Service::Http,
        _ => Service::Tcp,
    }
}

// Deliberately no Debug/Serialize: passwords are UI-only, in memory.
pub struct Connection {
    pub service: Service,
    pub user: String,
    pub password: String,
    pub database: String,
    pub auth_source: String,
    pub tls: bool,
    pub reveal: bool,
    pub note: String,
}
impl Connection {
    pub fn new(service: Service, port: u16) -> Self {
        Self { service, user: String::new(), password: String::new(), database: if service == Service::Amqp { "/".into() } else { String::new() }, auth_source: String::new(), tls: matches!(port, 9440 | 5671 | 8443), reveal: false, note: "Enter credentials, or read supported container environment values. Login has not been tested.".into() }
    }
    pub fn apply_env(&mut self, env: BTreeMap<String, String>) {
        let get = |key: &str| env.get(key).cloned().unwrap_or_default();
        let (u, p, d) = match self.service {
            Service::Postgres => {
                let u = env
                    .get("POSTGRES_USER")
                    .cloned()
                    .unwrap_or("postgres".into());
                let d = env.get("POSTGRES_DB").cloned().unwrap_or_else(|| u.clone());
                (u, get("POSTGRES_PASSWORD"), d)
            }
            Service::Mysql => {
                let prefix = if env.keys().any(|k| k.starts_with("MARIADB_")) {
                    "MARIADB"
                } else {
                    "MYSQL"
                };
                let u = get(&format!("{prefix}_USER"));
                let p = get(&format!("{prefix}_PASSWORD"));
                if !u.is_empty() && !p.is_empty() {
                    (u, p, get(&format!("{prefix}_DATABASE")))
                } else {
                    (
                        "root".into(),
                        get(&format!("{prefix}_ROOT_PASSWORD")),
                        get(&format!("{prefix}_DATABASE")),
                    )
                }
            }
            Service::Mongo => {
                self.auth_source = "admin".into();
                (
                    get("MONGO_INITDB_ROOT_USERNAME"),
                    get("MONGO_INITDB_ROOT_PASSWORD"),
                    get("MONGO_INITDB_DATABASE"),
                )
            }
            Service::Redis => (
                String::new(),
                env.get("VALKEY_PASSWORD")
                    .or_else(|| env.get("REDIS_PASSWORD"))
                    .cloned()
                    .unwrap_or_default(),
                String::new(),
            ),
            Service::ClickHouse | Service::ClickHouseHttp => (
                get("CLICKHOUSE_USER"),
                get("CLICKHOUSE_PASSWORD"),
                get("CLICKHOUSE_DB"),
            ),
            Service::Amqp => (
                get("RABBITMQ_DEFAULT_USER"),
                get("RABBITMQ_DEFAULT_PASS"),
                env.get("RABBITMQ_DEFAULT_VHOST")
                    .cloned()
                    .unwrap_or("/".into()),
            ),
            Service::S3 => (
                get("MINIO_ROOT_USER"),
                get("MINIO_ROOT_PASSWORD"),
                String::new(),
            ),
            _ => return,
        };
        self.user = u;
        self.password = p;
        self.database = d;
        self.note = "Read from container environment; may be initialization values. Password files, custom configs and rotated credentials are not detected. Login has not been tested.".into();
    }
    pub fn uri(&self, address: SocketAddr, include_password: bool) -> String {
        if self.service == Service::Tcp {
            return address.to_string();
        }
        if self.service == Service::S3 {
            return format!("{}://{address}/", if self.tls { "https" } else { "http" });
        }
        let scheme = match self.service {
            Service::Postgres => "postgresql",
            Service::Mysql => "mysql",
            Service::Mongo => "mongodb",
            Service::Redis if self.tls => "rediss",
            Service::Redis => "redis",
            Service::ClickHouse => "clickhouse",
            Service::Amqp if self.tls => "amqps",
            Service::Amqp => "amqp",
            Service::Https => "https",
            Service::ClickHouseHttp if self.tls => "https",
            _ => "http",
        };
        let mut auth = encode(&self.user);
        if include_password && !self.password.is_empty() {
            auth.push(':');
            auth.push_str(&encode(&self.password));
        }
        if !auth.is_empty() {
            auth.push('@');
        }
        let path = if self.service == Service::ClickHouseHttp {
            String::new()
        } else {
            encode(&self.database)
        };
        let query = match self.service {
            Service::ClickHouseHttp if !self.database.is_empty() => {
                format!("?database={}", encode(&self.database))
            }
            Service::Mongo if !self.auth_source.is_empty() => {
                format!("?authSource={}", encode(&self.auth_source))
            }
            Service::Postgres if self.tls => "?sslmode=require".into(),
            Service::Mongo if self.tls => "?tls=true".into(),
            Service::ClickHouse if self.tls => "?secure".into(),
            _ => String::new(),
        };
        let query = if self.service == Service::Mongo && self.tls && !self.auth_source.is_empty() {
            format!("{query}&tls=true")
        } else {
            query
        };
        format!("{scheme}://{auth}{address}/{path}{query}")
    }
    pub fn s3_config(&self, address: SocketAddr, include_password: bool) -> String {
        serde_json::json!({"endpoint":self.uri(address,false),"accessKeyId":self.user,"secretAccessKey":if include_password {self.password.as_str()} else {""},"forcePathStyle":true}).to_string()
    }
}
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

pub fn credentials_command(item: &Item, service: Service, sudo: bool) -> Result<String> {
    item.valid()
        .ok_or_else(|| anyhow!("Invalid container identity"))?;
    if item.runtime == Runtime::Kubernetes || service.env_keys().is_empty() {
        bail!("No environment credential preset for this service. Enter credentials manually.");
    }
    // Only allowlisted values leave the SSH server; arbitrary Env stays remote.
    let mut template = String::from("{{range .Config.Env}}{{$v := .}}");
    for key in service.env_keys() {
        template.push_str(&format!("{{{{if eq (index (split $v \"=\") 0) \"{key}\"}}}}{{{{json $v}}}}{{{{println}}}}{{{{end}}}}"));
    }
    template.push_str("{{end}}");
    Ok(format!(
        "{}{} inspect --format {} {}",
        sudo_prefix(sudo),
        item.runtime.cli(),
        shell_quote(&template),
        item.scope
    ))
}
pub fn parse_credentials(text: &str, service: Service) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let value: String =
            serde_json::from_str(line).map_err(|_| anyhow!("Invalid credential response"))?;
        let (key, value) = value
            .split_once('=')
            .ok_or_else(|| anyhow!("Invalid credential response"))?;
        if !service.env_keys().contains(&key) || value.len() > 16384 {
            bail!("Unexpected credential field");
        }
        values.insert(key.to_owned(), value.to_owned());
    }
    Ok(values)
}
pub async fn credentials(
    client: &SshClient,
    item: &Item,
    service: Service,
    sudo: bool,
) -> Result<BTreeMap<String, String>> {
    let command = credentials_command(item, service, sudo)?;
    let (text, code) =
        tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 128 * 1024))
            .await
            .map_err(|_| anyhow!("Credential request timed out"))?
            .map_err(|_| anyhow!("Could not read container credentials"))?;
    if code != 0 {
        bail!("Could not read container credentials. Check runtime access and sudo settings.");
    }
    parse_credentials(&text, service)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn address() -> SocketAddr {
        "127.0.0.1:43210".parse().unwrap()
    }
    #[test]
    fn image_detection_distinguishes_service_ports_and_unknown_images() {
        for (image, port, expected) in [
            ("registry:5000/team/postgres:17", 5432, Service::Postgres),
            ("mariadb@sha256:abc", 3306, Service::Mysql),
            ("valkey/valkey:8", 6379, Service::Redis),
            ("clickhouse/clickhouse-server:25", 9000, Service::ClickHouse),
            (
                "clickhouse/clickhouse-server:25",
                8123,
                Service::ClickHouseHttp,
            ),
            ("rabbitmq:management", 5672, Service::Amqp),
            ("rabbitmq:management", 15672, Service::Http),
            ("minio/minio:latest", 9000, Service::S3),
            ("minio/minio:latest", 9001, Service::Http),
            ("team/not-postgres:17", 5432, Service::Tcp),
            ("nginx:1", 443, Service::Https),
        ] {
            assert_eq!(detect(image, port), expected, "{image}:{port}");
        }
        assert_eq!(detect("postgres:17", 5432).mode(), OpenAs::Tcp);
    }
    #[test]
    fn credentials_and_paths_are_encoded_and_plain_copy_omits_password() {
        let mut c = Connection::new(Service::Postgres, 5432);
        c.user = "some@user".into();
        c.password = "p:/?#% é".into();
        c.database = "a/b ?".into();
        assert_eq!(
            c.uri(address(), true),
            "postgresql://some%40user:p%3A%2F%3F%23%25%20%C3%A9@127.0.0.1:43210/a%2Fb%20%3F"
        );
        assert_eq!(
            c.uri(address(), false),
            "postgresql://some%40user@127.0.0.1:43210/a%2Fb%20%3F"
        );
        c.tls = true;
        assert!(c.uri(address(), true).ends_with("?sslmode=require"));
        assert!(
            c.uri("[::1]:40000".parse().unwrap(), false)
                .contains("@[::1]:40000/")
        );
    }
    #[test]
    fn protocol_specific_uris_and_s3_config_keep_distinct_semantics() {
        let mut c = Connection::new(Service::Amqp, 5671);
        c.user = "app".into();
        c.password = "a=b".into();
        assert_eq!(
            c.uri(address(), true),
            "amqps://app:a%3Db@127.0.0.1:43210/%2F"
        );
        c = Connection::new(Service::Mongo, 27017);
        c.apply_env(BTreeMap::from([
            ("MONGO_INITDB_ROOT_USERNAME".into(), "admin".into()),
            ("MONGO_INITDB_ROOT_PASSWORD".into(), "pass".into()),
            ("MONGO_INITDB_DATABASE".into(), "events".into()),
        ]));
        c.tls = true;
        assert_eq!(
            c.uri(address(), true),
            "mongodb://admin:pass@127.0.0.1:43210/events?authSource=admin&tls=true"
        );
        c = Connection::new(Service::ClickHouseHttp, 8123);
        c.user = "analyst".into();
        c.database = "events/data".into();
        assert_eq!(
            c.uri(address(), false),
            "http://analyst@127.0.0.1:43210/?database=events%2Fdata"
        );
        c = Connection::new(Service::ClickHouse, 9440);
        assert_eq!(
            c.uri(address(), false),
            "clickhouse://127.0.0.1:43210/?secure"
        );
        c = Connection::new(Service::Redis, 6379);
        c.password = "pass".into();
        c.database = "0".into();
        assert_eq!(c.uri(address(), true), "redis://:pass@127.0.0.1:43210/0");
        c = Connection::new(Service::S3, 9000);
        c.user = "key".into();
        c.password = "secret".into();
        assert_eq!(c.uri(address(), true), "http://127.0.0.1:43210/");
        let config: serde_json::Value =
            serde_json::from_str(&c.s3_config(address(), true)).unwrap();
        assert_eq!(config["secretAccessKey"], "secret");
        assert_eq!(config["forcePathStyle"], true);
        assert_eq!(config["endpoint"], "http://127.0.0.1:43210/");
    }
    #[test]
    fn environment_mapping_does_not_pair_application_user_with_root_password() {
        let mut c = Connection::new(Service::Mysql, 3306);
        c.apply_env(BTreeMap::from([
            ("MYSQL_USER".into(), "app".into()),
            ("MYSQL_ROOT_PASSWORD".into(), "root-pass".into()),
        ]));
        assert_eq!((&*c.user, &*c.password), ("root", "root-pass"));
        c.apply_env(BTreeMap::from([
            ("MYSQL_USER".into(), "app".into()),
            ("MYSQL_PASSWORD".into(), "app-pass".into()),
            ("MYSQL_ROOT_PASSWORD".into(), "root-pass".into()),
        ]));
        assert_eq!((&*c.user, &*c.password), ("app", "app-pass"));
        c = Connection::new(Service::Postgres, 5432);
        c.apply_env(BTreeMap::from([("POSTGRES_USER".into(), "app".into())]));
        assert_eq!((&*c.user, &*c.database, &*c.password), ("app", "app", ""));
        assert!(c.note.contains("Login has not been tested"));
    }
    #[test]
    fn credential_response_filters_fields_and_preserves_equals_and_newlines() {
        let text = "\"POSTGRES_USER=app\"\n\"POSTGRES_PASSWORD=a=b\\nline\"\n";
        let values = parse_credentials(text, Service::Postgres).unwrap();
        assert_eq!(
            values.get("POSTGRES_PASSWORD").map(String::as_str),
            Some("a=b\nline")
        );
        for bad in ["not json", "\"API_TOKEN=private\"", "\"POSTGRES_USER\""] {
            assert!(parse_credentials(bad, Service::Postgres).is_err());
        }
        let item=super::super::parse("@docker\n{\"ID\":\"abcdef123456\",\"Names\":\"db\",\"Image\":\"postgres:17\",\"State\":\"running\"}\n").items.remove(0);
        let command = credentials_command(&item, Service::Postgres, true).unwrap();
        assert!(command.starts_with("sudo -n docker inspect --format '"));
        assert!(command.contains("eq (index (split $v \"=\") 0) \"POSTGRES_PASSWORD\""));
        assert!(!command.contains("{{json .Config.Env}}"));
        let mut hostile = item;
        hostile.scope = "abc; echo bad".into();
        assert!(credentials_command(&hostile, Service::Postgres, false).is_err());
    }
}
