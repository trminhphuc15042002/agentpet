# Personality Packs + Agent Party (T1) , Design Spec

Ngày: 2026-09-25
Trạng thái: Approved (brainstorm)
Phạm vi: **chỉ app Windows (Tauri + TypeScript)**. Không đụng macOS trong đợt này.

## 1. Tóm tắt

Hai update, làm theo thứ tự, đều nằm trong ràng buộc "gọn nhẹ máy":

1. **Personality Packs** , biến pool lời thoại hardcode thành dữ liệu (pack), để mỗi pet có giọng riêng. Thuần dữ liệu, không asset mới, không timer mới, không gọi mạng/AI.
2. **Agent Party T1** , cho người dùng thấy "đang có bao nhiêu subagent chạy và đứa nào cần mình", vẽ **trong pet window hiện tại**, không mở thêm cửa sổ/webview.

Ngoài phạm vi (đã chốt bỏ/park): Pet Journal, Habitat/Postcard, Quest/Bond, Away Companion/Mobile mirror.

## 2. Nguyên tắc gọn nhẹ (bắt buộc, dùng làm tiêu chí nghiệm thu)

| # | Nguyên tắc | Kiểm chứng |
|---|---|---|
| G1 | Không thêm cửa sổ hay webview mới | Đếm `WebviewWindow`/window count trước & sau; phải bằng nhau |
| G2 | Không thêm timer/poll định kỳ mới | `grep -n "setInterval\|scheduledTimer\|Timer"` trong `windows/src` không tăng |
| G3 | Không bundle thêm art/âm thanh | Kích thước bundle không tăng đáng kể; personality là JSON text |
| G4 | Không gọi LLM / không gọi mạng cho 2 feature này | Không có `fetch`/`invoke` mới ra ngoài cho personality/party |
| G5 | Idle CPU & RSS không tăng đo được | Task Manager: RSS lúc idle chênh ≤ ~10 MB, CPU idle ~0% |

## 3. Thứ tự / PR

| PR | Nội dung | Loại thay đổi |
|---|---|---|
| PR1 | Personality Packs: data model + 3 pack built-in + Settings selector + i18n | TS thuần |
| PR2 | Agent Party: plumbing subagent roster (Rust + state.ts) | Rust + TS, chưa có UI |
| PR3 | Agent Party: badge trên pet + roster trong bubble/list | TS thuần |

PR2 tách khỏi PR3 để nếu dữ liệu subagent không đủ tin cậy thì UI không bị block.

---

## 4. PR1 , Personality Packs

### 4.1 Mục tiêu

Cùng một event, mỗi pet phản ứng khác nhau về **giọng điệu, tần suất nói, và cách ăn mừng** , mà không cần animation frame mới.

### 4.2 Không làm

- Không thêm mood/animation state mới. Giữ nguyên tập mood hiện có: `idle/working/waiting/done/celebrate`.
- Không đụng `ACTIVITY_THEMES` (chef/engineer/wizard/...). Đó là **vocabulary** (từ vựng công việc), khác trục với personality.
- Không cho người dùng tải pack từ mạng ở PR1 (local/typed-in only). Pack cộng đồng để sau.

### 4.3 Phân biệt hai trục (quan trọng)

- **Vocabulary** (`ap_theme_phrases`): "đang làm gì" , `Compiling…`, `Brewing…`.
- **Personality** (`ap_personality`): "nói thế nào, khi nào, mừng ra sao" , lời thoại reactive, idle chatter, tần suất, emoji, âm thanh.

Hai trục độc lập, chọn riêng trong Settings, nhân với nhau khi render.

### 4.4 Data model

Pack là một object trong bộ built-in, có thể override bằng JSON người dùng dán vào Settings (PR1 chỉ cần built-in; custom JSON là tùy chọn nếu rẻ).

```ts
export type PersonalityID = "cozy" | "tsundere" | "chaotic";

export interface PersonalityPack {
  id: PersonalityID;
  /// Multiplier cho ngưỡng cooldown: >1 = nói ít hơn, <1 = nói nhiều hơn.
  /// Chỉ áp lên cooldown, KHÔNG áp lên ngưỡng metric (tránh phá cảm giác tiến bộ).
  chattiness: number;
  /// Pool theo metric + tier, thay cho PHRASES trong reactive.ts.
  reactive: Record<string, string[]>;
  /// Lời thay cho PET_CHAT / IDLE_BOOST khi pack bật.
  chat: Partial<Record<"working" | "waiting" | "done" | "celebrate" | "idle", string[]>>;
  /// Chọn âm thanh có sẵn theo event (không thêm file âm thanh mới).
  sound?: Partial<Record<"done" | "waiting" | "celebrate" | "levelup", string>>;
}
```

Ba pack built-in (giọng tiếng Anh, khớp style hiện tại; chuỗi hiển thị đi qua `t()` như hiện nay):

| Pack | chattiness | Ví dụ `done` | Ví dụ khi quota/hunger xấu |
|---|---|---|---|
| **cozy** (mặc định) | 1.0 | "All done, go stretch ☕" | "Let's slow down a little." |
| **tsundere** | 0.7 | "Fine, it works. I wasn't worried." | "You burned it all again. Obviously." |
| **chaotic** | 1.3 | "SHIP IT 🚀" | "Houston, we have a token problem." |

### 4.5 Điểm tích hợp (đường dẫn chính xác)

| File | Hiện tại | Thay đổi |
|---|---|---|
| `windows/src/reactive.ts` | `PHRASES` hằng số (dòng 20–35) | Đọc `reactive` từ pack đang chọn; giữ nguyên cấu trúc `pool()`/`checkCooldown()` |
| `windows/src/reactive.ts` | `TH.cooldown` cố định (dòng 16) | Nhân `sameMetric`/`crossMetric` với `1/chattiness` khi check; kẹp biên để không spam |
| `windows/src/activity.ts` | `PET_CHAT`, `IDLE_BOOST`, `defaultLines()` (dòng 161–199) | `defaultLines()` trả pool của pack khi `ap_msg_src = system`; custom messages vẫn thắng như hiện tại |
| `windows/src/settings.ts` | đọc `ap_theme_phrases` (dòng 705–707) | Thêm select personality, key `ap_personality`, default `cozy` |
| `windows/settings.html` | group "Activity messages" (dòng 360–372) | Thêm group "Personality" cạnh đó |
| `windows/src/i18n.ts` | dict vi/zh/zh-TW | Thêm key cho tên pack + mô tả + nhãn |

Quan hệ ưu tiên khi render bubble (giữ nguyên thứ tự hiện có): **custom messages > pack.chat > PET_CHAT mặc định**.

### 4.6 UI

Settings → group **Personality**:

- Select: Cozy / Tsundere / Chaotic.
- Dòng caption: "Giọng điệu pet dùng khi tự nói. Không ảnh hưởng từ vựng hoạt động."
- Nút **Preview** (tùy chọn): hiện một câu mẫu ngay trong dòng caption , không mở cửa sổ.

### 4.7 Tiêu chí nghiệm thu PR1

- Đổi pack → câu reactive và idle chatter đổi giọng ngay, không cần restart.
- Custom bubble messages vẫn thắng pack.
- G1–G5 giữ nguyên.
- `ap_reactive = 0` vẫn tắt hoàn toàn reactive như cũ.

---

## 5. PR2 , Agent Party data (subagent roster)

### 5.1 Vấn đề thực tế trong code

App **chưa biết subagent nào đang chạy**. Bằng chứng:

- `windows/src-tauri/src/hooks.rs` (dòng 38, 40, 44, 56): danh sách event có `SubagentStop` nhưng **không có `SubagentStart`**.
- `windows/src-tauri/src/statemap.rs` (dòng 26, 42): Claude/Droid `SubagentStop` → `None` (chỉ dùng để tính token).
- `windows/src-tauri/src/server.rs` (dòng 209–226): `SubagentStop` chỉ đọc transcript để feed XP.
- `windows/src/state.ts`: `Session` không có field subagent.

Nên đây là **việc dữ liệu trước, UI sau**.

### 5.2 Cách suy ra subagent (không phụ thuộc mỗi agent một kiểu)

Vì nhiều agent không có hook "start" cho subagent, dùng quy tắc suy luận:

1. **Mở**: `PreToolUse` với tool `Task`/`Agent` (Claude/Droid/Codex) → một subagent bắt đầu, lấy `subagent` id nếu payload có, else sinh id tạm theo `session + timestamp`.
2. **Đóng**: `SubagentStop` (đã có) với `subagent` id → gỡ khỏi roster.
3. **Nếu agent có `SubagentStart`** (Codex đã được map ở `statemap.rs:30` nhưng chưa đăng ký hook) → đăng ký thêm và ưu tiên event này để mở.
4. **An toàn**: subagent không có `Stop` sẽ bị dọn theo timeout (mặc định 5 phút, cùng tinh thần `STALE_ACTIVE_MS` trong `state.ts`).
5. **Session kết thúc** (`SessionEnd`) → xóa sạch roster của session đó.

### 5.3 Thay đổi dữ liệu

| File | Thay đổi |
|---|---|
| `statemap.rs` | Thêm `SubagentStart` cho Claude/Droid nếu agent thực sự phát; nếu không, để `state()` trả `None` (không đổi mood) và xử lý roster ở tầng server |
| `server.rs` | Khi nhận event subagent: cập nhật roster của session (thêm/gỡ), giữ tối đa N (vd 8) để chặn phình bộ nhớ |
| `hooks.rs` | Bổ sung `SubagentStart` vào danh sách event của agent hỗ trợ (Codex), giữ nguyên các agent khác |
| `cli.rs` | Không đổi , field `subagent` đã có (dòng 103, 256–269) |
| `state.ts` | `Session.subagents: { id: string; role: string; startedAt: number }[]`; cập nhật trong `update()`; dọn trong `active()` cùng session cha |
| `state.ts` | `aggregateMood()` không đổi (subagent không tự tạo mood riêng) |

### 5.4 Tiêu chí nghiệm thu PR2

- Chạy một prompt Claude Code có `Task` → roster session đó có ≥1 subagent ngay khi task bắt đầu.
- Khi subagent xong → roster giảm.
- Subagent "mồ côi" biến mất sau timeout, không rò rỉ qua nhiều session.
- `state.ts` không giữ quá N subagent/session.

---

## 6. PR3 , Agent Party UI (T1)

### 6.1 Nguyên tắc render

Vẽ trong surface hiện có, tận dụng pattern đã có thay vì surface mới:

- **Badge trên pet**: pet surface thêm một badge nhỏ số subagent đang chạy + chấm màu "cần bạn" khi có subagent/session `waiting`.
- **Roster gọn**: tái dùng danh sách session trong `bubble.ts` (mỗi row đã có icon slot, title, project, state, clock, và badge nhóm `.xn`). Thêm inline `👥 N` cho session có subagent; không dựng list hay panel mới.
- **Badge trên pet**: hiển thị `👥 N` cạnh canvas pet hiện có. Đây là tín hiệu T1; role-level roster / icon vai trò để T2 khi event data đủ tin cậy.

### 6.2 Không làm (đẩy sang T2/T3)

- Không mini-pet sprite di chuyển riêng (đắt: mỗi con là window/webview + 60fps).
- Không animation riêng cho từng subagent.
- Không thêm cửa sổ dashboard.

### 6.3 Tiêu chí nghiệm thu PR3

- Chạy ≥2 subagent song song → badge hiện đúng số, roster đúng từng dòng.
- Một subagent `waiting` → pet có tín hiệu "cần bạn" đúng thành viên đó.
- Click row subagent → focus terminal như row session hiện tại (best-effort, giữ hành vi cũ).
- Có setting tắt (mặc định bật) để người không quan tâm không thấy badge.

---

## 7. Đo lường & bằng chứng

Trước PR1 và sau PR3, ghi lại:

- RSS lúc idle (Task Manager) , chênh mục tiêu ≤ ~10 MB.
- CPU idle , mục tiêu ~0%.
- Số window/webview , phải không đổi.
- Danh sách timer trong `windows/src` , không tăng.

Bằng chứng dán vào PR: ảnh chụp Task Manager trước/sau + output grep timer.

## 8. Rủi ro

| Rủi ro | Mức | Giảm thiểu |
|---|---|---|
| Suy luận subagent bằng `Task` bắt nhầm tool khác | Trung bình | Chỉ nhận tool name chính xác; `SubagentStop` là nguồn đóng; timeout dọn |
| Roster phình theo session dài | Thấp | Giới hạn N/session + xóa khi `SessionEnd` |
| Personality làm pet nói quá nhiều | Thấp | `chattiness` chỉ nhân cooldown, có kẹp biên; cooldown hiện tại vẫn giữ |
| Trùng lặp với custom messages | Thấp | Giữ nguyên thứ tự ưu tiên hiện có |

## 9. Việc đã chốt KHÔNG làm

Pet Journal (trùng agentmemory/Obsidian), Habitat/Postcard, Quest/Bond, Away Companion/Mobile mirror.
