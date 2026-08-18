PY ?= /home/bartoszm/miniconda3/bin/python3

install:
	ln -sfn $(CURDIR) $(HOME)/.claude/skills/capture-one
	@echo "installed as ~/.claude/skills/capture-one"

test:
	$(PY) -m pytest tests/ -q

check:
	$(PY) $(HOME)/.agents/skills/skill-creator/scripts/quick_validate.py skills/capture-one-catalog
	$(PY) $(HOME)/.agents/skills/skill-creator/scripts/quick_validate.py skills/capture-one-vision
	claude plugin validate $(CURDIR) --strict

eval:
	claude plugin eval capture-one --ablation with-without --allow-tools 'Bash' --no-publish

sync:
	./bin/c1cat sync

.PHONY: install test check eval sync
