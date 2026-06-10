//! Stage-1 前處理:把 STT 逐字稿清成乾淨文字,再交給 board-agent(stage 2)。
//! 規則層(scrub_fillers)只動「高確信」贅字;錯字 / 斷句 / 語意層交給 LLM
//! (cleanup_transcript,prompt 在 prompts/transcript-cleanup.md,失敗 fallback 規則層結果)。
//! 同 mori-ear cleanup.rs 的兩段式精神:清稿永遠不該擋住後面的建卡。
use crate::llm::{self, Msg};

/// 連續重複的字 / 雙字詞收斂;每個句段開頭的語助詞移除。保守:只動高確信贅字。
pub fn scrub_fillers(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    // 1) 連續同字 3+ 次 → 1 次(疊字詞如「謝謝」「常常」是兩次,不動)
    let mut pass1: Vec<char> = Vec::with_capacity(chars.len());
    for &c in &chars {
        let n = pass1.len();
        if n >= 2 && pass1[n - 1] == c && pass1[n - 2] == c && is_cjk(c) {
            continue;
        }
        pass1.push(c);
    }
    // 口語應答詞連續兩次也收斂成一次(白名單,避免誤傷一般疊字詞)
    let mut s: String = pass1.into_iter().collect();
    for d in ["對對", "好好", "是是", "恩恩", "嗯嗯"] {
        let single: String = d.chars().take(1).collect();
        while s.contains(d) {
            s = s.replace(d, &single);
        }
    }
    // 2) 連續重複的雙字詞(那個那個 / 就是就是)→ 一次
    let cs: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(cs.len());
    let mut i = 0;
    while i < cs.len() {
        if i + 4 <= cs.len()
            && cs[i] == cs[i + 2]
            && cs[i + 1] == cs[i + 3]
            && cs[i] != cs[i + 1]
            && is_cjk(cs[i])
            && is_cjk(cs[i + 1])
        {
            out.push(cs[i]);
            out.push(cs[i + 1]);
            i += 4;
            while i + 2 <= cs.len() && cs[i] == out[out.len() - 2] && cs[i + 1] == out[out.len() - 1]
            {
                i += 2;
            }
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    let s: String = out.into_iter().collect();
    // 3) 每個句段開頭的語助詞(嗯呃欸喔哦啊)+ 跟隨的標點/空白移除
    const FILLER_HEAD: [char; 6] = ['嗯', '呃', '欸', '喔', '哦', '啊'];
    s.split_inclusive(['。', '!', '?', '!', '?', ';', ';', ',', ',', '\n'])
        .map(|seg| {
            let lead: String = seg
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            let mut t = seg.trim_start();
            loop {
                let mut it = t.chars();
                match it.next() {
                    Some(c) if FILLER_HEAD.contains(&c) => {
                        t = it
                            .as_str()
                            .trim_start_matches([',', ',', '、', ' '])
                            .trim_start();
                    }
                    _ => break,
                }
            }
            format!("{}{}", lead, t)
        })
        .collect::<Vec<_>>()
        .concat()
        // 清掉「贅字刪光只剩標點」的空句段
        .replace(",,", ",")
        .replace("。。", "。")
        .trim_start_matches([',', ',', '。'])
        .to_string()
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// 整段逐字稿太長時切成 ~1500 字塊逐塊清(LLM context / 速度),完成後接回。
fn cleanup_blocks(raw: &str) -> Vec<String> {
    const MAX: usize = 1500;
    let mut blocks = vec![];
    let mut cur = String::new();
    for unit in raw.split_inclusive(['\n', '。', '!', '?', '!', '?']) {
        if !cur.is_empty() && cur.chars().count() + unit.chars().count() > MAX {
            blocks.push(std::mem::take(&mut cur));
        }
        cur.push_str(unit);
    }
    if !cur.trim().is_empty() {
        blocks.push(cur);
    }
    blocks
}

/// Stage-1 清稿:規則層 → LLM 層。LLM 失敗就用規則層結果(永不擋住 stage 2)。
/// 回傳 (清好的文字, 是否套到 LLM 層)。輸入太短(< 10 字,多半是 UI 指令)只過規則層。
pub async fn cleanup_transcript(raw: &str, local_only: bool, opts: &llm::LlmOpts) -> (String, bool) {
    let scrubbed = scrub_fillers(raw);
    if scrubbed.chars().count() < 10 {
        return (scrubbed, false);
    }
    // lang=en:放寬語言規則、嚴禁翻譯 —— 清稿輸出保持講者原語言(見 EN_CLEANUP_DIRECTIVE)
    let sys = llm::with_cleanup_lang(crate::prompts::prompt("transcript-cleanup"), opts.lang);
    let mut cleaned = String::new();
    for block in cleanup_blocks(&scrubbed) {
        let msgs = [
            Msg {
                role: "system",
                content: sys.clone(),
            },
            Msg {
                role: "user",
                content: block.clone(),
            },
        ];
        match llm::chat(&msgs, false, local_only, opts).await {
            Ok((t, _)) => {
                let t = crate::strip_think(&t);
                if !t.trim().is_empty() {
                    cleaned.push_str(t.trim());
                    cleaned.push('\n');
                }
            }
            Err(_) => return (scrubbed, false), // LLM 掛了 → 整段用規則層結果
        }
    }
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        (scrubbed, false)
    } else {
        (cleaned, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_repeated_chars_and_words() {
        assert_eq!(scrub_fillers("對對對對,就這樣"), "對,就這樣");
        assert_eq!(scrub_fillers("那個那個我們先報價"), "那個我們先報價");
        assert_eq!(scrub_fillers("然後然後下週交付"), "然後下週交付");
    }

    #[test]
    fn strips_leading_interjections_per_segment() {
        assert_eq!(scrub_fillers("嗯,我們下週交付"), "我們下週交付");
        assert_eq!(scrub_fillers("呃 這個案子。欸,先做 demo"), "這個案子。先做 demo");
    }

    #[test]
    fn keeps_meaningful_text_untouched() {
        let s = "客戶擔心櫃台人員不會用後台,我說會做教學影片。";
        assert_eq!(scrub_fillers(s), s);
        // 正常疊字詞(兩次)不可動
        assert_eq!(scrub_fillers("謝謝大家常常支持"), "謝謝大家常常支持");
    }

    #[test]
    fn blocks_split_on_sentence_ends_around_1500_chars() {
        let raw = "句子。".repeat(900); // 2700 字
        let blocks = cleanup_blocks(&raw);
        assert!(blocks.len() >= 2);
        assert!(blocks.iter().all(|b| b.chars().count() <= 1503));
        assert_eq!(blocks.concat(), raw);
    }
}
