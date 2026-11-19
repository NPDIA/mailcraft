# MailCraft 기여 안내

> [영문 CONTRIBUTING.md](CONTRIBUTING.md)의 한국어 번역입니다. 기준 원문의 상태: 현행 · 작성 2026-10-15 · 갱신 2026-11-14 · 변경 유형 minor. 아래는 **원본 프로젝트에 기여할 때의 규칙**입니다. 실제 제출 전에는 최신 영문 지침을 따릅니다.

MailCraft는 사용자의 메일을 보관하는 프로그램입니다. 기여 내용만큼이나 기여가 들어오는 과정이 중요합니다.

## 작업을 시작하기 전에

1. [작업 지침](AGENTS.ko.md)([영문](AGENTS.md))을 읽습니다. 중단을 일으키지 않고, 메일은 비공개로 다루되 내용은 신뢰하지 않으며, 타사 자산을 쓰지 않고 클린룸 방식으로 작업합니다. 사람과 에이전트에 모두 적용되며 예외는 없습니다.
2. 베타 로드맵은 [#2](https://github.com/UCS-Care-Group/mailcraft/issues/2)에서 확인합니다. 채택된 결정에 어긋난 작업은 잘 구현해도 병합되지 않습니다. 먼저 결정의 변경을 제안합니다.
3. 댓글에서 아직 맡은 사람이 없는 열린 이슈에 “I'm taking this”라고 댓글을 달고 시작합니다. 별도의 담당자 배정을 기다리지 않습니다. `claimed` 라벨은 표시일 뿐 승인 절차가 아닙니다. 첫 댓글의 신청이 우선이므로 기존 댓글을 읽고, 함께 작업하려면 담당자에게 먼저 알립니다. 담당자가 14일간 댓글·PR·초안을 올리지 않으면 다른 사람이 댓글로 이어받을 수 있습니다. 이슈에 없는 새 기능이나 변경은 이슈를 열고 동의를 받은 뒤 시작합니다.
4. 열린 PR은 검토를 기다리는 동안 기간에 관계없이 작성자가 기능을 맡고 있음을 뜻합니다. 검토에서 변경을 요청하면 작성자는 7일 안에 커밋이나 댓글로 답해야 합니다. 답이 없으면 담당권이 풀리고, 다른 사람이 이슈를 맡아 공개 명세로부터 독립적으로 다시 구현할 수 있습니다. 기존 PR의 코드는 사용하지 않습니다. 다른 구현이 들어오지 않았다면 기존 PR도 여전히 업데이트·병합할 수 있습니다.
5. 버그와 수정안을 모두 준비했다면 [문제 보고](https://github.com/UCS-Care-Group/mailcraft/issues/new?template=problem.yml)와 수정 PR을 같이 올리고, PR에 `Closes #<issue>`를 적습니다. 따로 작업을 맡겠다고 신청하거나 기다릴 필요는 없습니다. 수정안 없이 문제만 보고해도 됩니다. 다만 [소유자 결정 사항](#소유자-결정-사항)에 해당하면 먼저 이슈에서 묻습니다.

`post-beta` 이슈는 베타에 도달할 때까지 담당 신청을 받지 않습니다.

## PR

- 첫 PR에서 사용자가 [기여자 라이선스 계약](CLA.md)을 한 번 읽고 동의한다면 `I have read the MailCraft CLA and I agree to it.`이라고 직접 댓글을 달아 서명합니다. 기여자는 저작권을 보유하며 UCS Care Group Limited가 다른 조건으로 공개할 권리를 부여합니다. MailCraft에는 계속 MIT OR Apache-2.0이 적용됩니다. 모든 작성자가 서명하면 `cla` 검사가 통과합니다.
- 현재 `main`을 기준으로 이슈마다 PR 하나를 작성하며, 한 번에 검토할 수 있을 만큼 작게 나눕니다. PR 설명에 `Closes #<issue>`를 씁니다.
- 여러 부분으로 나뉜 기능은 부분별 PR을 앞선 부분 위에 올리고 설명 첫 줄에 `Builds on #<n>`을 적습니다. 뒤쪽 PR은 초안으로 두고, 앞부분이 병합되면 다음 PR을 `main`에 맞춘 후 검토 준비로 바꿉니다. 전체를 함께 검토해야 한다면 첫 PR에 명시합니다. 유지보수자는 묶음을 함께 검토하여 최상단 PR을 병합하고 나머지는 포함된 것으로 닫을 수 있습니다. 이 저장소에서는 포크 PR을 서로의 브랜치에 직접 연결할 수 없어 설명에서 순서를 표시합니다.
- **유지보수자의 수정 허용**을 켭니다(포크 기본값). 유지보수자가 브랜치를 `main`에 맞추거나 작은 수정을 직접 할 수 있습니다.
- 모든 커밋 전에 로컬에서 `cargo xtask ci`를 통과합니다. 빠른 검사(fmt, assets, spdx, test-files, workflows, layers, wasm-clock: 브라우저 경로에서 std 시계 사용 금지, i18n --check, release-notes, docs-log) 뒤에 Clippy, 테스트, wasm과 설치된 경우 cargo-deny가 실행됩니다. 메시지에 작업 ID를 넣습니다(예: `R3.2: messages come down by delta`).
- 문서 개정 이력은 `docs/log.d/<document without .md>/revisions/<issue>.md`, `ROADMAP.md` 진행 기록은 `docs/log.d/ROADMAP/progress/<issue>.md`, 릴리스 노트는 `packaging/release-notes.d/`의 별도 파일로 작성합니다. 표와 릴리스 노트를 직접 편집하지 않습니다. 릴리스 시 합쳐져 PR끼리 충돌하지 않습니다. [문서 기록](docs/log.d/README.md), [릴리스 노트 기록](packaging/release-notes.d/README.md)을 참조하세요.
- 프로그램 중단 및 데이터 손실 수정마다 회귀 테스트를 추가합니다.
- 보안 지적을 수정할 때는 보고된 한 곳뿐 아니라 원인이 같은 모든 지점을 검사합니다. 해당 유형의 모든 명령, 읽기·쓰기 구현을 목록으로 시험하고, 나중에 추가된 구현에 같은 수정이 누락되면 테스트가 실패하도록 합니다.
- 테스트는 `tests.rs`, `<name>_tests.rs`, `tests/`에 두고 `#[cfg(test)] mod <name>_tests;`로 포함합니다. 소스에 인라인 `mod tests { … }`를 추가하지 않습니다. [코드 스캔](.github/codeql/codeql-config.yml)이 별도 파일만 제외하기 때문에 소스의 가짜 비밀번호와 검증 메시지도 실제 코드처럼 탐지됩니다. `cargo xtask test-files`는 새 인라인 모듈을 거부합니다. 이전 목록 `xtask/inline-tests.txt`는 해당 테스트를 분리할 때 그 줄만 지우고, 새 줄을 넣지 않습니다.
- 새 Rust 파일은 두 줄의 SPDX 저작권·라이선스 헤더로 시작합니다(`cargo xtask spdx --fix`).
- 실제 사서함으로 테스트하지 않습니다. `crates/net/src/fake.rs`의 가짜 서버와 Graph 대역, `example.org` 주소를 사용합니다.
- 동기화, 저장소, 로그인, 비밀정보를 바꾸면 [CODEOWNERS](.github/CODEOWNERS)에 따라 병합 전에 유지보수자가 검토합니다.

## 지속적 통합(CI)

PR은 `ci-ok`와 `cla`를 통과해야 합니다.

- **PR:** `quick`이 먼저 실행됩니다. 서식, SPDX, 카탈로그, 조각 파일 등을 검사하여 오래된 `.po`나 합치지 못하는 조각을 초기에 발견합니다. 변경 패키지와 이에 의존하는 모든 패키지는 Linux·macOS·Windows에서 시험합니다. `cargo xtask affected`가 변경 파일을 nextest 필터로 만들며 실행 로그·요약에 선택 범위와 이유가 나옵니다. 번역만 변경한 경우(`crates/i18n/po/`와 문서)는 Linux에서만 실행합니다. Android 앱 파일만 변경하면 Android 작업만 실행하고 Rust 테스트는 실행하지 않습니다. 문서만 변경하면 `quick`만 실행합니다. Android 바인딩 검사에는 그 빌드에 필요한 파일의 변경이 감지되어야 합니다.
- **전체 검사:** 모든 패키지에 영향을 주는 파일(`Cargo.lock`, 최상위 `Cargo.toml`, 툴체인, `.cargo/`, `.config/`, `xtask/`, `.github/workflows/`, `.github/scripts/`)이나 어느 패키지에도 속하지 않는 파일이 바뀐 PR, `main` 푸시 및 수동 실행에서 모든 운영체제를 시험합니다. 병합 큐에서는 macOS 실행기가 적어 Linux로 전체 검사를 하고, 병합 후 `main` 푸시에서 모든 운영체제를 확인합니다.
- PR에 `full-ci` 라벨을 추가하면 전체 검사 실행이 시작됩니다.
- **정기 검사:** 커버리지(야간 및 `main`; PR에서는 하지 않음), FreeBSD와 Windows Arm(야간), 모든 읽기 기능 퍼징(매주 및 읽기 코드 변경 PR), 감사, 벤치마크가 있습니다.

PR에 선택된 검사만으로는 `main`에서도 통과할지 보장할 수 없습니다. 의존성에 드러나지 않는 파일을 읽는 테스트가 있을 수 있습니다. `main`은 병합할 때마다 모든 검사를 실행하며, 확신이 없으면 라벨로 전체 검사를 실행합니다.

## 첫 기여

[`good first issue`](https://github.com/UCS-Care-Group/mailcraft/labels/good%20first%20issue)는 범위가 작고 볼 파일이 명시되어 있습니다. 이슈에 댓글을 달아 맡을 수 있습니다.

## PR 검토 기준

[거버넌스 문서의 변경 검토 기준](GOVERNANCE.md#how-changes-are-reviewed)을 따릅니다.

## 소유자 결정 사항

다음은 이슈에서 먼저 묻고 동의를 얻기 전에는 PR에 넣지 않습니다.

- Microsoft 또는 다른 제공업체에 새 권한을 요청하는 변경
- HTML 표시 허용 목록(`crates/html/src/style.rs`)에 항목을 추가하는 변경
- `AGENTS.md`(`CLAUDE.md`가 연결), `PRIVACY.md`, `TERMS.md` 또는 채택된 설계 결정 변경
- 새 크레이트, MIT·Apache-2.0·BSD·ISC 이외 라이선스의 새 의존성, 또는 새 자산
- 현재 컴퓨터 밖으로 보내지 않는 정보를 새로 전송하는 변경

## MailCraft 테스트

직접 사용해 보려면 [테스터 양식](https://github.com/UCS-Care-Group/mailcraft/issues/new?template=tester-signup.yml)으로 신청하세요. 문제는 [문제 보고](https://github.com/UCS-Care-Group/mailcraft/issues/new?template=problem.yml)로 접수합니다.

이 저장소의 기본 빌드에서 도움말 › 문제 보고 › 보고서 보내기는 실제 릴레이(`.cargo/config.toml`의 `MAILCRAFT_REPORT_URL`)로 전송되며, 유지보수자가 보는 이슈가 생성됩니다. 화면 테스트를 위해 실제 전송을 누르지 않습니다. `MAILCRAFT_REPORT_URL=`(빈 값)으로 빌드하면 GitHub에서 열기만 표시되며, 전송 자체는 `crates/net/src/report_tests.rs`의 릴레이 대역으로 시험합니다.

## 개정 이력

| 날짜 | 변경 유형 | 요약 |
|---|---|---|
| 2026-11-14 | patch | `cargo xtask ci`의 wasm-clock 단계: 브라우저 코드에서 std 시계·sleep 금지(#1337) |
| 2026-11-12 | minor | 열린 PR의 기능 담당권은 검토 후 7일간 답변을 기다리고, 그 뒤에는 다시 구현 가능 |
| 2026-11-11 | patch | #956 3부: 실제 보고 릴레이 전송과 화면 시험용 `MAILCRAFT_REPORT_URL=` 안내 |
| 2026-11-07 | minor | 보안 수정은 원인이 같은 모든 지점을 테스트 |
| 2026-11-04 | minor | 문서와 진행 기록 개정 줄을 릴리스 때 합치는 별도 조각으로 관리 |
| 2026-11-03 | minor | 댓글로 이슈 담당 신청, 첫 신청 우선, 14일간 응답이 없으면 만료 |
| 2026-10-27 | minor | 기능을 부분별 PR로 나누는 방법, `Builds on #<n>`, 초안 및 유지보수자 수정 |
| 2026-10-24 | minor | 새 인라인 테스트 모듈 금지 검사와 기존 목록 |
| 2026-10-23 | minor | 버그는 별도 신청 없이 문제 보고와 수정 PR을 함께 제출 |
| 2026-10-21 | minor | 코드 스캔 제외를 위해 테스트를 별도 파일로 분리 |
| 2026-10-20 | minor | 첫 기여, PR 검토, 새 Rust 파일의 SPDX 헤더 |
| 2026-10-19 | minor | [기여자 라이선스 계약](CLA.md)을 PR 댓글로 한 번 직접 서명 |
| 2026-10-16 | minor | 동기화·저장소·로그인·비밀정보의 유지보수자 검토(강제된 코드 소유자 검토 보장은 없음), `CLAUDE.md` 연결 |
| 2026-10-15 | major | 최초 버전: 작업 먼저 맡기, 이슈당 PR 하나, 소유자 결정, 테스터 신청 |
