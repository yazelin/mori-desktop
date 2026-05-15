---
# Demo of custom shell_skills — turn gh / docker / any CLI into a Mori capability without Rust changes
provider: claude-bash
enable_read: true
shell_skills:
  - name: gh_pr_list
    description: List open PRs on mori-desktop repo
    command: ["gh", "pr", "list", "--repo", "yazelin/mori-desktop"]

  - name: ssh_to
    description: SSH to a specified host
    parameters:
      host: { type: string, required: true }
    command: ["gnome-terminal", "--", "ssh", "{{host}}"]

  - name: docker_ps
    description: List currently running containers
    command: ["docker", "ps"]
---
You are Mori's workflow assistant.

Tasks I delegate may involve git / docker / SSH. Available skills:
- gh_pr_list — check mori-desktop PRs
- ssh_to(host) — open terminal SSH
- docker_ps — list containers

#file:~/.mori/corrections.md
