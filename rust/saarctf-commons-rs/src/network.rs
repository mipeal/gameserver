//! Port of `saarctf_commons/config.py` `NetworkConfig` — the team↔IP math.

use serde_json::Value;

pub type IpComponent = (i64, i64, i64);

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkConfig {
    pub vulnbox_ip: Vec<IpComponent>,
    pub gateway_ip: Vec<IpComponent>,
    pub testbox_ip: Vec<IpComponent>,
    pub network_ip: Vec<IpComponent>,
    pub vpn_peer_ip: Vec<IpComponent>,
    pub network_size: i64,
    pub gameserver_ip: Option<String>,
}

/// `((team_id / a) mod b) + c` per component, dotted — the shared shape of all
/// `team_id_to_*` conversions. Python `//` and `%` floor toward negative
/// infinity, so use the euclidean ops.
fn apply(components: &[IpComponent], team_id: i64) -> String {
    components
        .iter()
        .map(|&(a, b, c)| (team_id.div_euclid(a).rem_euclid(b) + c).to_string())
        .collect::<Vec<_>>()
        .join(".")
}

impl NetworkConfig {
    /// `parse_network_def`: each component is a number `-> (1, 1, n)` or a 3-list.
    pub fn parse_network_def(x: &[Value]) -> Result<Vec<IpComponent>, String> {
        x.iter()
            .map(|c| match c {
                Value::Number(n) => {
                    let n = n
                        .as_i64()
                        .ok_or_else(|| format!("invalid network component: {n}"))?;
                    Ok((1, 1, n))
                }
                Value::Array(list) => {
                    if list.len() != 3 {
                        return Err(format!("expected 3 components, got {}", list.len()));
                    }
                    let mut out = [0i64; 3];
                    for (i, v) in list.iter().enumerate() {
                        out[i] = v
                            .as_i64()
                            .ok_or_else(|| format!("invalid network component: {v}"))?;
                    }
                    Ok((out[0], out[1], out[2]))
                }
                other => Err(format!("invalid network component: {other}")),
            })
            .collect()
    }

    pub fn from_dict(d: &Value) -> Result<NetworkConfig, String> {
        let get = |key: &str| -> Result<&Value, String> {
            d.get(key).ok_or_else(|| format!("KeyError: '{key}'"))
        };
        let list = |key: &str| -> Result<&Vec<Value>, String> {
            get(key)?
                .as_array()
                .ok_or_else(|| format!("'{key}' must be a list"))
        };
        let team_range = list("team_range")?;
        if team_range.len() < 5 {
            return Err("team_range needs 5 components".into());
        }
        let network_size = team_range[4]
            .as_i64()
            .ok_or_else(|| "team_range[4] must be an int".to_string())?;
        if !matches!(network_size, 8 | 16 | 24 | 32) {
            return Err(format!(
                "ValueError: Team network size {network_size} unsupported"
            ));
        }
        Ok(NetworkConfig {
            vulnbox_ip: Self::parse_network_def(list("vulnbox_ip")?)?,
            gateway_ip: Self::parse_network_def(list("gateway_ip")?)?,
            testbox_ip: Self::parse_network_def(list("testbox_ip")?)?,
            network_ip: Self::parse_network_def(&team_range[..4])?,
            vpn_peer_ip: Self::parse_network_def(list("vpn_peer_ips")?)?,
            network_size,
            gameserver_ip: d
                .get("gameserver_ip")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        })
    }

    pub fn to_dict(&self) -> Value {
        let list = |ips: &[IpComponent]| -> Value {
            Value::Array(
                ips.iter()
                    .map(|(a, b, c)| Value::Array(vec![(*a).into(), (*b).into(), (*c).into()]))
                    .collect(),
            )
        };
        let mut d = serde_json::Map::from_iter([
            ("vulnbox_ip".into(), list(&self.vulnbox_ip)),
            ("gateway_ip".into(), list(&self.gateway_ip)),
            ("testbox_ip".into(), list(&self.testbox_ip)),
            ("vpn_peer_ips".into(), list(&self.vpn_peer_ip)),
            (
                "team_range".into(),
                Value::Array(
                    list(&self.network_ip)
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned()
                        .chain([self.network_size.into()])
                        .collect(),
                ),
            ),
        ]);
        if let Some(ip) = &self.gameserver_ip {
            d.insert("gameserver_ip".into(), ip.clone().into());
        }
        Value::Object(d)
    }

    pub fn team_id_to_vulnbox_ip(&self, team_id: i64) -> String {
        apply(&self.vulnbox_ip, team_id)
    }

    pub fn team_id_to_gateway_ip(&self, team_id: i64) -> String {
        apply(&self.gateway_ip, team_id)
    }

    pub fn team_id_to_testbox_ip(&self, team_id: i64) -> String {
        apply(&self.testbox_ip, team_id)
    }

    pub fn team_id_to_network_range(&self, team_id: i64) -> String {
        format!("{}/{}", apply(&self.network_ip, team_id), self.network_size)
    }

    pub fn team_id_to_vpn_peers(&self, team_id: i64) -> (String, String) {
        let mut second = self.vpn_peer_ip.clone();
        if let Some(last) = second.last_mut() {
            last.2 += 1;
        }
        (apply(&self.vpn_peer_ip, team_id), apply(&second, team_id))
    }

    /// Inverse of `team_id_to_network_range`'s host part: find the team id whose
    /// network contains `ip`, or `None` (the interval walk of the original).
    pub fn network_ip_to_id(&self, ip: &str) -> Option<i64> {
        let octets: Vec<i64> = ip.split('.').filter_map(|p| p.parse().ok()).collect();
        let mut a: Vec<i64> = Vec::new();
        let mut b: Vec<i64> = Vec::new();
        let mut pos: Vec<i64> = Vec::new();
        for (i, &(ai, bi, ci)) in self
            .network_ip
            .iter()
            .take((self.network_size / 8) as usize)
            .enumerate()
        {
            if bi > 1 {
                a.push(ai);
                b.push(bi);
                pos.push((octets.get(i).copied().unwrap_or(0) - ci) * ai);
            }
        }
        loop {
            let smallest = *pos.iter().max()?;
            let largest = pos.iter().zip(a.iter()).map(|(p, a)| p + a).min()?;
            if smallest >= 0xFFFF {
                return None;
            }
            if smallest < largest {
                return Some(smallest);
            }
            for i in 0..pos.len() {
                while pos[i] + a[i] <= smallest {
                    pos[i] += a[i] * b[i];
                }
            }
        }
    }
}
