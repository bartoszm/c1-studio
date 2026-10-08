---
max_turns: 20
timeout_seconds: 1200
allowed_tools: [Bash, Read, Glob, Grep]
runs: 1
tags: [slow, costly]
---
Pick 12 photos from my Capture One catalog that are rated 5 stars, build a contact sheet
from them in ./sheets, and have Gemini tell me what is in them. Do not write anything back
to the catalog.
