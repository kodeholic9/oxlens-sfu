// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-KEYSPACE · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 맹검 독립 1층 — sfud 도 키 조각이 못 되는 node_id · 배포 이름공간이면 기동 실패다.

use oxsfud::cfg::SfuCfg;

fn args(id: &str, ns: &str) -> Vec<String> {
    let policy = format!("{}/../../policy.toml", env!("CARGO_MANIFEST_DIR"));
    ["--id", id, "--policy", &policy, "--zenoh-namespace", ns, "--grpc-listen", "127.0.0.1:0", "--udp-port", "0", "--zenoh-connect", "tcp/127.0.0.1:7447", "--node-grace-secs", "10"].iter().map(|s| s.to_string()).collect()
}

#[test]
fn unusable_node_id_fails_sfud_boot() {
    let ok = SfuCfg::load(args("node-a", "ox"));
    assert!(ok.is_ok(), "S-sym-RULE-BUS-KEYSPACE-6 control: sfud loads with plain values · {:?}", ok.err());
    for bad in ["", "n/1", "n*", "n$", "n?", "n#"] {
        assert!(SfuCfg::load(args(bad, "ox")).is_err(), "S-sym-RULE-BUS-KEYSPACE-6 sfud with node id {bad:?} must fail at boot");
    }
}

#[test]
fn unusable_namespace_fails_sfud_boot() {
    let mut accepted = Vec::new();
    for bad in ["", "n/1", "n*", "n$", "n?", "n#"] {
        if SfuCfg::load(args("node-a", bad)).is_ok() {
            accepted.push(bad);
        }
    }
    assert!(accepted.is_empty(), "S-sym-RULE-BUS-KEYSPACE-6 sfud accepted unusable namespaces {accepted:?}");
}
