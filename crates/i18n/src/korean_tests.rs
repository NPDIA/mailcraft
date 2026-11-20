// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::{Lang, date};

#[test]
fn korean_codes_name_the_same_preview_language() {
    let korean = Lang::Korean;
    assert_eq!(korean.code(), "ko");
    assert_eq!(korean.native_name(), "한국어");
    assert!(korean.preview());
    for code in ["ko", "ko-KR", "KO_kr.UTF-8"] {
        assert_eq!(Lang::from_code(code), Some(korean), "{code}");
    }
}

#[test]
fn korean_is_offered_only_when_its_catalogue_and_platform_support_it() {
    let korean = Lang::Korean;
    assert_eq!(korean.supported_on(false), korean.has_catalogue());
    assert!(!korean.supported_on(true));
    let available = korean.has_catalogue() && !cfg!(target_arch = "wasm32");
    let expected = if available { korean } else { Lang::English };
    assert_eq!(Lang::available().contains(&korean), available);
    assert_eq!(Lang::from_pref("ko", None), expected);
    assert_eq!(Lang::from_pref("auto", Some("ko-KR")), expected);
}

#[test]
fn korean_uses_a_twelve_hour_clock_and_a_sunday_start() {
    let dated = date::Dated::new(Lang::Korean, Some("KR"));
    assert!(dated.conventions.twelve_hour);
    assert_eq!(dated.conventions.week_start, 6);
    assert!(!dated.conventions.pad_numeric);
}

#[test]
fn the_korean_catalogue_formats_dates_and_times() {
    let korean = Lang::Korean;
    if !korean.has_catalogue() {
        return;
    }
    let day = date::Day { year: 2026, month: 10, day: 4, weekday: 6 };
    assert_eq!(date::long(&korean, day), "2026년 10월 4일 일요일");
    assert_eq!(date::numeric(&korean, day), "2026. 10. 4.");
    assert_eq!(date::time(&korean, 9, 5, true), "오전 9:05");
    assert_eq!(date::time(&korean, 21, 5, true), "오후 9:05");
}

#[test]
fn korean_names_the_spam_folder_and_distinguishes_rule_conditions_from_times() {
    let korean = Lang::Korean;
    assert_eq!(korean.folder("Junk Email"), "스팸 메일");
    assert_eq!(korean.tc("rule condition", "When"), "조건");
    assert_eq!(korean.t("When"), "시각");
}
