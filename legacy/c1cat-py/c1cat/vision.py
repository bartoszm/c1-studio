"""Delegating visual analysis to Gemini through the Antigravity CLI (`agy`).

Result extraction deliberately does not rely on `agy --json-schema`. With a schema
attached, agy answers in prose and then runs a separate turn to fill the schema;
that turn was observed returning an empty object while the prose answer was
correct. So the prose channel is the primary one and the schema is only a hint.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import time
from pathlib import Path

AGY = Path.home() / ".local" / "bin" / "agy"

TASKS = ("cull", "describe", "keyword", "geo", "all")

RESULT_SCHEMA = {
    "type": "object",
    "properties": {
        "photos": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "label": {"type": "string"},
                    "keep": {"type": "boolean"},
                    "rating": {"type": "integer"},
                    "description": {"type": "string"},
                    "headline": {"type": "string"},
                    "keywords": {"type": "array", "items": {"type": "string"}},
                    "keywords_new": {"type": "array", "items": {"type": "string"}},
                    "location_guess": {"type": "object"},
                    "confidence": {"type": "number"},
                    "reject_reason": {"type": "string"},
                },
                "required": ["label"],
            },
        }
    },
    "required": ["photos"],
}


def agy_path() -> Path | None:
    if AGY.exists():
        return AGY
    found = shutil.which("agy")
    return Path(found) if found else None


# --------------------------------------------------------------------------
# prompts
# --------------------------------------------------------------------------

COMMON = """You are looking at a contact sheet of photographs from a photographer's
Capture One catalog. Each cell has a label printed directly underneath it, of the
form "v<number> <filename>". Use that label verbatim to identify each photo.

Rules:
- Emit exactly one object per photo you can actually see on the sheet.
- Copy the label exactly as printed. Never invent or guess a label.
- Omit any field you cannot judge rather than filling it with a guess.
- Reply with a single fenced ```json block and nothing else.
"""

TASK_PROMPTS = {
    "cull": """For each photo decide whether it is worth keeping.
Give "keep" (true/false), a "rating" from 1 to 5, a short "reject_reason" when
keep is false, and a "confidence" from 0 to 1.
Judge on: is the subject sharp where it matters, is the moment or gesture strong,
is the composition deliberate, is the exposure recoverable.
""",
    "describe": """For each photo write:
- "description": two or three sentences describing what is happening, the light
  and the mood, as a curator would write for a portfolio.
- "headline": at most five words.
- "confidence": 0 to 1.
""",
    "keyword": """For each photo choose keywords.
Prefer keywords from this photographer's existing vocabulary, listed below. Put any
genuinely new keyword in "keywords_new" instead of "keywords".
Give "keywords" (existing ones you chose), "keywords_new" (at most three), and
"confidence".

Existing vocabulary:
{vocabulary}
""",
    "geo": """For each photo, if you recognise the place, give
"location_guess": {"city": ..., "country": ..., "confidence": 0-1}.
Only answer where a landmark, sign or unmistakable architecture supports it.
Leave it out entirely when you are not sure.
""",
}

RESPONSE_SHAPE = """
Expected shape:
```json
{"photos": [{"label": "v273331 DSCF4365", "rating": 4, "keywords": ["street"],
             "description": "...", "confidence": 0.8}]}
```
"""


def build_prompt(task: str, sheet: Path, vocabulary: list[str] | None = None,
                 language: str = "en") -> str:
    if task == "all":
        body = "\n".join(TASK_PROMPTS[t] for t in ("cull", "describe", "keyword", "geo"))
    else:
        body = TASK_PROMPTS[task]
    if "{vocabulary}" in body:
        body = body.replace("{vocabulary}", ", ".join(vocabulary or []) or "(none yet)")
    language_note = ""
    if language == "pl":
        language_note = "\nWrite every description, headline and caption in Polish.\n"
    elif language == "both":
        language_note = ("\nWrite 'description' in English and add 'description_pl' "
                         "with the Polish version.\n")
    return (f"Read the image at {sheet}\n\n{COMMON}\n{body}{language_note}{RESPONSE_SHAPE}")


# --------------------------------------------------------------------------
# running agy
# --------------------------------------------------------------------------

_FENCE = re.compile(r"```(?:json)?\s*(.*?)```", re.S)


def extract_json(text: str) -> dict | None:
    """structured_output is unreliable, so parse the prose. Fences first, then braces."""
    if not text:
        return None
    for candidate in reversed(_FENCE.findall(text)):
        try:
            return json.loads(candidate.strip())
        except ValueError:
            continue
    start = text.find("{")
    while start != -1:
        depth = 0
        for i in range(start, len(text)):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    try:
                        return json.loads(text[start:i + 1])
                    except ValueError:
                        break
        start = text.find("{", start + 1)
    return None


EFFORT_SUFFIXES = ("-low", "-medium", "-high")


def run_sheet(sheet: Path, prompt: str, *, model: str = "gemini-3.1-pro-low",
              effort: str | None = None, timeout: int = 600) -> dict:
    binary = agy_path()
    if binary is None:
        raise FileNotFoundError(
            "the Antigravity CLI (agy) was not found at ~/.local/bin/agy or on PATH")
    argv = [
        str(binary), "--print", prompt,
        "--add-dir", str(sheet.parent),
        "--model", model,
        "--output-format", "json",
        "--print-timeout", f"{timeout}s",
    ]
    # Most agy model names carry the effort level as a suffix, and passing --effort
    # alongside one of those is rejected outright ("conflicts with --effort").
    if effort and not model.endswith(EFFORT_SUFFIXES):
        argv += ["--effort", effort]
    started = time.time()
    proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout + 60)
    duration = round(time.time() - started, 1)

    envelope: dict = {}
    if proc.stdout.strip():
        try:
            envelope = json.loads(proc.stdout)
        except ValueError:
            envelope = {"status": "UNPARSEABLE_ENVELOPE", "response": proc.stdout}

    response = envelope.get("response", "")
    structured = envelope.get("structured_output")
    parsed = structured if isinstance(structured, dict) and structured.get("photos") else None
    if parsed is None:
        parsed = extract_json(response)

    # agy reports usage errors in an "error" field with an empty response, so surface
    # that rather than leaving the caller with a blank failure.
    error = envelope.get("error") or ""

    return {
        "ok": proc.returncode == 0 and envelope.get("status") == "SUCCESS" and bool(parsed),
        "returncode": proc.returncode,
        "status": envelope.get("status"),
        "error": error,
        "conversation_id": envelope.get("conversation_id"),
        "duration_s": envelope.get("duration_seconds", duration),
        "usage": envelope.get("usage", {}),
        "response": response or error or proc.stderr,
        "parsed": parsed,
    }


# --------------------------------------------------------------------------
# validation
# --------------------------------------------------------------------------

def validate(parsed: dict, label_map: dict[str, int], known_keywords: set[str],
             min_confidence: float = 0.0) -> tuple[list[dict], list[str]]:
    """Map the model's answers onto variant PKs and drop anything unusable."""
    items, warnings = [], []
    photos = parsed.get("photos") if isinstance(parsed, dict) else parsed
    if not isinstance(photos, list):
        return [], ["response contained no 'photos' array"]

    for entry in photos:
        if not isinstance(entry, dict):
            continue
        label = str(entry.get("label", "")).strip()
        variant_pk = label_map.get(label)
        if variant_pk is None:
            # Tolerate a model that echoed only the pk or only the filename.
            token = label.split()[0] if label else ""
            variant_pk = label_map.get(token)
        if variant_pk is None:
            warnings.append(f"unknown label {label!r} - dropped")
            continue

        item: dict = {"variant_pk": variant_pk, "label": label}
        confidence = entry.get("confidence")
        item["confidence"] = float(confidence) if isinstance(confidence, (int, float)) else None
        if item["confidence"] is not None and item["confidence"] < min_confidence:
            item["below_confidence"] = True

        if isinstance(entry.get("rating"), (int, float)):
            item["rating"] = max(0, min(5, int(entry["rating"])))
        if isinstance(entry.get("keep"), bool):
            item["keep"] = entry["keep"]
        for field in ("description", "headline", "reject_reason"):
            if isinstance(entry.get(field), str) and entry[field].strip():
                item[field] = entry[field].strip()

        accepted, proposed = [], []
        for raw in entry.get("keywords", []) or []:
            if not isinstance(raw, str):
                continue
            keyword = raw.strip().lower()
            if not keyword or "," in keyword:
                warnings.append(f"rejected keyword {raw!r} (a comma would corrupt the catalog)")
                continue
            (accepted if keyword in known_keywords else proposed).append(keyword)
        for raw in entry.get("keywords_new", []) or []:
            if isinstance(raw, str) and raw.strip() and "," not in raw:
                proposed.append(raw.strip().lower())
        if accepted:
            item["keywords"] = sorted(set(accepted))
        if proposed:
            item["keywords_proposed"] = sorted(set(proposed))

        guess = entry.get("location_guess")
        if isinstance(guess, dict) and (guess.get("city") or guess.get("country")):
            item["location_guess"] = {
                "city": guess.get("city"), "country": guess.get("country"),
                "confidence": guess.get("confidence"),
            }
        items.append(item)
    return items, warnings


# --------------------------------------------------------------------------
# batch state
# --------------------------------------------------------------------------

def load_tracker(batch: Path) -> dict:
    return json.loads((batch / "tracker.json").read_text())


def save_tracker(batch: Path, tracker: dict) -> None:
    (batch / "tracker.json").write_text(json.dumps(tracker, indent=2, ensure_ascii=False) + "\n")


def merged_items(batch: Path) -> list[dict]:
    """All validated items across every completed sheet, newest answer per variant."""
    out: dict[int, dict] = {}
    for path in sorted((batch / "results").glob("sheet_*.json")):
        for item in json.loads(path.read_text()).get("items", []):
            out[item["variant_pk"]] = item
    return list(out.values())
