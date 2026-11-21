// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
//! 채팅 3rd 구독 매칭 — 원천 `server/areas/chat/behavior/extsub.ts` 의 거울(벡터 `chat.json` 이 맞댄다): 계정 `subscribe` 항목 꼴(모든 종류 · 글 · 접두 · 종류 하나 — 꼴이 틀리면 설정 읽기 실패) · 메시지 종류가 먼저 · 원본을 고치거나 지우는 관계와 발언 기록 갱신은 원본 종류도 · 판단은 갈래와 걸린 항목을 사유로 낸다 · 계정 여럿의 표.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubItem {
    All,
    Text,
    Prefix(String),
    Kind(String),
}

impl SubItem {
    pub fn parse(s: &str) -> Option<SubItem> {
        if s == "*" {
            return Some(SubItem::All);
        }
        if s.is_empty() {
            return Some(SubItem::Text);
        }
        if let Some(p) = s.strip_suffix('*')
            && s.ends_with(".*")
        {
            return (p.len() > 1 && !p.contains('*')).then(|| SubItem::Prefix(p.to_string()));
        }
        (!s.contains('*')).then(|| SubItem::Kind(s.to_string()))
    }

    pub fn fits(&self, kind: &str) -> bool {
        match self {
            SubItem::All => true,
            SubItem::Text => kind.is_empty(),
            SubItem::Prefix(p) => kind.starts_with(p.as_str()),
            SubItem::Kind(k) => kind == k,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Own,
    Orig,
}

impl By {
    pub fn as_str(self) -> &'static str {
        match self {
            By::Own => "own",
            By::Orig => "orig",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub by: By,
    pub item: usize,
}

pub fn follows(kind: &str) -> bool {
    matches!(kind, "edit" | "delete" | "talk_update")
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct Subscribe {
    raw: Vec<String>,
    items: Vec<SubItem>,
}

impl TryFrom<Vec<String>> for Subscribe {
    type Error = String;

    fn try_from(raw: Vec<String>) -> Result<Subscribe, String> {
        let mut items = Vec::with_capacity(raw.len());
        for s in &raw {
            items.push(SubItem::parse(s).ok_or_else(|| format!("subscribe 항목 {s:?} — \"*\" · \"\" · x.y.* · * 없는 종류 하나여야 한다"))?);
        }
        Ok(Subscribe { raw, items })
    }
}

impl Subscribe {
    pub fn raw(&self) -> &[String] {
        &self.raw
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn matches(&self, kind: &str, orig: Option<&str>) -> Option<Hit> {
        decide(self.items.len(), |i, k| self.items[i].fits(k), kind, orig)
    }
}

pub fn matches_raw(subscribe: &[String], kind: &str, orig: Option<&str>) -> Option<Hit> {
    let items: Vec<Option<SubItem>> = subscribe.iter().map(|s| SubItem::parse(s)).collect();
    decide(items.len(), |i, k| items[i].as_ref().is_some_and(|it| it.fits(k)), kind, orig)
}

fn decide(len: usize, fits: impl Fn(usize, &str) -> bool, kind: &str, orig: Option<&str>) -> Option<Hit> {
    let first = |k: &str| (0..len).find(|&i| fits(i, k));
    if let Some(item) = first(kind) {
        return Some(Hit { by: By::Own, item });
    }
    let o = orig.filter(|_| follows(kind))?;
    first(o).map(|item| Hit { by: By::Orig, item })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtSubs {
    accounts: Vec<(String, Subscribe)>,
}

impl ExtSubs {
    pub fn new(accounts: Vec<(String, Subscribe)>) -> ExtSubs {
        ExtSubs { accounts: accounts.into_iter().filter(|(_, s)| !s.is_empty()).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.accounts.iter().map(|(k, _)| k.as_str())
    }

    pub fn hits(&self, kind: &str, orig: Option<&str>) -> Vec<(&str, Hit)> {
        self.accounts.iter().filter_map(|(k, s)| s.matches(kind, orig).map(|h| (k.as_str(), h))).collect()
    }
}
