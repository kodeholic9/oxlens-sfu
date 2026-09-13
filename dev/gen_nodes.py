#!/usr/bin/env python3
# author: kodeholic (powered by Claude)
"""개발 실증 형상 생성기 — ★**node 를 손으로 세 벌 쓰지 않는다.**

★★**node 하나 = hub 하나 + sfu 하나**(정§15-0 — hub 가 router 를 임베드한다).
node 를 늘리는 일이 흔한데 파일을 손으로 복사하면 ★**포트 하나가 겹쳐도 조용히 죽는다.**
그래서 규칙을 여기 한 곳에 두고 파일은 산출물로 둔다.

    python3 dev/gen_nodes.py 3

포트 규칙 — ★**한 자리만 보면 겹침을 눈으로 안다.**
    hub HTTP   19745 + i
    sfu gRPC   50061 + i
    sfu UDP    20000 + i
    zenoh      7447  + i
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
N = int(sys.argv[1]) if len(sys.argv) > 1 else 3
NS = "oxdev"

AUTH = '''
[hub.auth]
jwt_secret = "change-me-in-production"

[[hub.auth.api_keys]]
key = "ox_k_demo"
secret = "ox_s_demo"
name = "demo"
# 정§3-4·§16-1-1 — 이 계정이 서명할 수 있는 것. 투명·운영은 켜는 것이 명시적 결정이다.
participant_types = [0, 1, 2]
hidden_allowed = true
ops_allowed = true

# 계정마다 낼 수 있는 것이 다르다(정§3-4) — 제한 계정이 있어야 `2005` 갈래가 실증된다.
[[hub.auth.api_keys]]
key = "ox_k_limited"
secret = "ox_s_limited"
name = "limited"
participant_types = [0]
hidden_allowed = false
ops_allowed = false
'''


def node_name(i: int) -> str:
    return f"node-{chr(ord('a') + i)}"


for i in range(N):
    me = node_name(i)
    # ★**나머지 전부에 연다** — seed 이고, 붙은 뒤엔 router 끼리 알아서 나른다.
    #   ★한 쪽만 걸면 그 쪽이 죽을 때 나머지가 서로를 못 본다.
    connect = [f'"tcp/127.0.0.1:{7447 + j}"' for j in range(N) if j != i]
    body = f'''# ★**생성물이다 — 손으로 고치지 않는다**(`dev/gen_nodes.py {N}`).
#
# ★★**node 하나 = hub 하나 + sfu 하나**(정§15-0). 배치 키는 `node_id` 이고(정§15-3)
#   유닛 이름을 그 값과 같게 맞춘다 — 그래야 「방 → node」가 곧 「방 → 그 node 의 sfu」다.
# ★**배치는 `room_id` 만으로 정해진다**(HRW 순수 함수) — 그래서 방 이름을 고르면
#   ★**어느 node 로 갈지 미리 안다.** 시험이 결정적이 되는 자리가 이것이다.
[hub]
listen = "127.0.0.1:{19745 + i}"
base_path = "/media"
{AUTH}
# ★유닛은 hub 가 띄운다(정§16-1) — 하네스가 sfud 를 직접 띄우지 않는다.
[[unit]]
id = "{me}"
kind = "process"
order = 1
role = "sfu"
addr = "127.0.0.1:{50061 + i}"
cmd = ["./target/debug/oxsfud", "--grpc-listen", "127.0.0.1:{50061 + i}", "--udp-port", "{20000 + i}", "--policy", "policy.toml"]

# ★**이 node 의 버스**(정§15-0) — hub 는 router, 성립 조건 둘은 명시로 적는다.
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
    print(f"{out.name}  hub=127.0.0.1:{19745 + i}  sfu={me}@{50061 + i}  udp={20000 + i}  zenoh={7447 + i}")
