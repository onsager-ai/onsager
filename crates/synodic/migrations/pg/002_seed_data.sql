-- Seed: 10 threat categories from governance threat taxonomy (PostgreSQL)

INSERT INTO threat_categories (id, name, description, severity, severity_weight, examples) VALUES
    ('data-destruction', 'Data Destruction', 'Irreversible data loss (rm -rf, DROP TABLE, git reset --hard)', 'critical', 1.0,
     '["rm -rf /", "git reset --hard", "git clean -fdx", "DROP DATABASE"]'),
    ('credential-exposure', 'Credential Exposure', 'Secrets, tokens, keys exposed in code/logs/args', 'critical', 1.0,
     '["echo $API_KEY", "hardcoded passwords", ".env in commit"]'),
    ('system-integrity', 'System Integrity', 'Modifications to system files outside project scope', 'high', 0.7,
     '["write /etc/hosts", "modify /usr/bin/", "chmod 777 /"]'),
    ('supply-chain', 'Supply Chain', 'Dependency manipulation, malicious package install', 'high', 0.7,
     '["npm install unknown-pkg", "pip install from URL", "curl | bash"]'),
    ('network-exfiltration', 'Network Exfiltration', 'Unauthorized data transmission to external endpoints', 'high', 0.7,
     '["curl -d @/etc/passwd", "wget --post-file"]'),
    ('privilege-escalation', 'Privilege Escalation', 'Gaining elevated permissions', 'high', 0.7,
     '["sudo", "chmod u+s", "chown root"]'),
    ('persistence', 'Persistence Mechanisms', 'Creating backdoors, cron jobs, startup scripts', 'medium', 0.4,
     '["crontab -e", "write to .bashrc", "launchctl load"]'),
    ('git-history-manipulation', 'Git History Manipulation', 'Rewriting shared history, force-pushing', 'medium', 0.4,
     '["git rebase published", "git push --force to main"]'),
    ('resource-exhaustion', 'Resource Exhaustion', 'Consuming excessive CPU, memory, disk, or network', 'medium', 0.4,
     '["fork bomb", "while true", "dd if=/dev/zero"]'),
    ('ci-pipeline-manipulation', 'CI Pipeline Manipulation', 'Modifying CI/CD configs to bypass checks', 'medium', 0.4,
     '["edit .github/workflows", "disable test step"]')
ON CONFLICT DO NOTHING;

-- Seed: 5 default rules (the original hardcoded rules, now in DB)

INSERT INTO rules (id, description, category_id, tools, condition_type, condition_value, lifecycle, alpha, beta, prior_alpha, prior_beta, enabled) VALUES
    ('destructive-git', 'Block destructive git operations on protected branches', 'data-destruction',
     '["Bash"]', 'command', 'git\s+(reset\s+--hard|push\s+--force|push\s+-f|clean\s+-fd)\b',
     'active', 2, 1, 2, 1, TRUE),
    ('secrets-in-args', 'Block tool calls containing potential secrets', 'credential-exposure',
     '[]', 'pattern', '(?i)(api[_-]?key|secret[_-]?key|password|token)\s*[=:]\s*\S{8,}',
     'active', 2, 1, 2, 1, TRUE),
    ('writes-outside-project', 'Block file writes outside the project root', 'system-integrity',
     '["Write", "Edit"]', 'path', '/etc/**',
     'active', 2, 1, 2, 1, TRUE),
    ('writes-to-system', 'Block file writes to system directories', 'system-integrity',
     '["Write", "Edit"]', 'path', '/usr/**',
     'active', 2, 1, 2, 1, TRUE),
    ('dangerous-rm', 'Block rm -rf with root or home paths', 'data-destruction',
     '["Bash"]', 'command', 'rm\s+-[rR]f?\s+(/\s|/$|~/|~\s|~$|\$HOME)',
     'active', 2, 1, 2, 1, TRUE)
ON CONFLICT DO NOTHING;
