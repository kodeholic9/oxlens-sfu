#!/usr/bin/env python3
# author: kodeholic (powered by Claude)
# spec: v1.2 @124a233 · 정:IMPL-CFG-SYSTEM-FILE 정:RULE-BUS-SHAPE 정:RULE-OPS-LOGGER 운영 규격서 §7-1 · §8-1 · §8-3 · model: claude-opus-5-5
"""개발 실증 형상 생성기 — node 마다 시스템 파일 하나(hub 1974 부터 연속).

    python3 dev/gen_nodes.py 3

로그 — node 마다 로거 유닛 하나(정:RULE-OPS-LOGGER) · {LOG_ROOT}/{node}/ox.log.{날짜} · 소켓 {LOG_ROOT}/{node}/log.sock
포트 — hub HTTP 1974+i · sfu gRPC 50061+i · sfu UDP 20000+i · sfu TCP 20000+i(정:IMPL — TCP 전송로 · 설정됐을 때만 광고) · zenoh 7447+i
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
N = int(sys.argv[1]) if len(sys.argv) > 1 else 3
NS = "oxdev"
HUB_BASE = 1974
LOG_ROOT = "/tmp/oxlens"

AUTH = '''
[hub.auth]
jwt_secret = "change-me-in-production"

[[hub.auth.api_keys]]
key = "ox_k_demo"
secret = "ox_s_demo"
name = "demo"
participant_types = [0, 1, 2]
hidden_allowed = true
ops_allowed = true
hook_allowed = true

[[hub.auth.api_keys]]
key = "ox_k_limited"
secret = "ox_s_limited"
name = "limited"
participant_types = [0]
hidden_allowed = false
ops_allowed = false
hook_allowed = false
'''


def node_name(i: int) -> str:
    return f"node-{chr(ord('a') + i)}"


for i in range(N):
    me = node_name(i)
    connect = [f'"tcp/127.0.0.1:{7447 + j}"' for j in range(N) if j != i]
    logs = f"{LOG_ROOT}/{me}"
    body = f'''[hub]
listen = "127.0.0.1:{HUB_BASE + i}"
base_path = "/media"
log_dir = "{logs}"
log_socket = "{logs}/log.sock"
{AUTH}
[[unit]]
id = "log-{me}"
kind = "process"
order = 1
role = "log"
cmd = ["./target/debug/oxlogd", "--policy", "policy.toml"]

[[unit]]
id = "{me}"
kind = "process"
order = 1
role = "sfu"
addr = "127.0.0.1:{50061 + i}"
cmd = ["./target/debug/oxsfud", "--grpc-listen", "127.0.0.1:{50061 + i}", "--udp-port", "{20000 + i}", "--tcp-port", "{20000 + i}", "--policy", "policy.toml", "--public-ip", "auto"]

[zenoh]
mode = "router"
listen = ["tcp/127.0.0.1:{7447 + i}"]
connect = [{", ".join(connect)}]
multicast_scouting = false
qos_enabled = true
lowlatency = false
namespace = "{NS}"
'''
    out = HERE / f"system.{me}.toml"
    out.write_text(body, encoding="utf-8")
    print(f"{out.name}  hub=127.0.0.1:{HUB_BASE + i}  sfu={me}@{50061 + i}  udp={20000 + i}  tcp={20000 + i}  zenoh={7447 + i}")
