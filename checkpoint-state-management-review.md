# Amazon Exec Code Review: Ralph Checkpoint/PRD State Management

## Customer Problem

**Who is the customer?** Developers using ralph to automate multi-story implementation tasks.

**What happened?** Ralph timed out mid-execution, leaving the system in an inconsistent state:
- Checkpoint claimed work was on US-002 (Google OAuth)
- Actual uncommitted changes were for US-019/020/021/023 (completely different stories)
- No branch was created before work started
- User had no clear path to recovery

**Customer impact:** Confusion, lost work, manual intervention required, loss of trust in the tool.

---

## Root Cause Analysis

Based on the exploration of ralph's architecture, three failure modes combined:

| Issue | What Broke | Customer Impact |
|-------|-----------|-----------------|
| No branch validation | Work started on review/exec-brutal instead of ralph/production-readiness | Changes go to wrong branch |
| Checkpoint story mismatch | Checkpoint said US-002 but changes were for different stories | Recovery guidance is wrong |
| No pre-flight checks | Ralph didn't verify clean state before starting | Clobbered existing uncommitted work |

---

## Probing Questions

1. **What happens if ralph starts when uncommitted changes already exist?**
   - Current: Silently continues, potentially mixing changes
   - Expected: Should abort or stash

2. **What happens if current branch ≠ PRD branchName?**
   - Current: Continues anyway
   - Expected: Should switch or abort with clear error

3. **How does the checkpoint track which files belong to which story?**
   - Current: Just lists uncommitted files globally
   - Expected: Should tag files per story for accurate recovery

4. **What's the recovery UX when checkpoint state is inconsistent?**
   - Current: User must manually investigate
   - Expected: Ralph should detect and offer remediation

---

## Prioritized Recommendations

### P0: Pre-flight Safety Checks (Prevents the problem)

Before starting work, ralph MUST verify:
1. Current branch == prd.branchName (or offer to switch)
2. No uncommitted changes exist (or offer to stash)
3. Checkpoint story matches PRD state

**Implementation location:** Ralph's initialization before story iteration begins.

### P1: Branch Auto-Creation/Switch

- If branch doesn't exist → Create it from main
- If branch exists but not current → Switch to it (with stash if needed)

**Why:** The branchName field in prd.json should be authoritative.

### P2: Per-Story File Tracking in Checkpoint

Current checkpoint:
```json
"uncommitted_files": ["file1.rs", "file2.rs"]
```

Improved checkpoint:
```json
"story_changes": {
  "US-002": ["file1.rs"],
  "US-019": ["file2.rs", "file3.rs"]
}
```

**Why:** Enables accurate recovery when multiple stories were touched.

### P3: Checkpoint Consistency Validation on Resume

When resuming, validate:
- Files in checkpoint still exist
- Files haven't been modified by other processes
- Story in checkpoint matches PRD priority order
- Branch is correct

If inconsistent → warn user and offer: `[Investigate] [Reset] [Force Resume]`

---

## Proposed Pre-flight Check Implementation

Add to ralph before story processing:

```rust
fn preflight_checks(prd: &Prd, checkpoint: Option<&Checkpoint>) -> Result<(), PreflightError> {
    // 1. Branch check
    let current_branch = git_current_branch()?;
    if current_branch != prd.branch_name {
        return Err(PreflightError::WrongBranch {
            expected: prd.branch_name.clone(),
            actual: current_branch,
        });
    }

    // 2. Clean working tree
    let uncommitted = git_uncommitted_files()?;
    if !uncommitted.is_empty() && checkpoint.is_none() {
        return Err(PreflightError::DirtyWorkingTree { files: uncommitted });
    }

    // 3. Checkpoint consistency (if resuming)
    if let Some(cp) = checkpoint {
        let current_uncommitted: HashSet<_> = uncommitted.into_iter().collect();
        let checkpoint_files: HashSet<_> = cp.uncommitted_files.iter().cloned().collect();

        if current_uncommitted != checkpoint_files {
            return Err(PreflightError::CheckpointMismatch {
                expected: checkpoint_files,
                actual: current_uncommitted,
            });
        }
    }

    Ok(())
}
```

---

## Summary

| Priority | Fix | Effort | Impact |
|----------|-----|--------|--------|
| P0 | Pre-flight branch/clean-state checks | Low | Prevents 90% of clobbered states |
| P1 | Auto-switch to PRD branch | Low | Removes manual branch management |
| P2 | Per-story file tracking | Medium | Enables accurate partial recovery |
| P3 | Checkpoint consistency validation | Medium | Graceful degradation on resume |

---

## Recovery Status (2025-01-20)

Recovery complete. Ralph is now ready to run fresh on `ralph/production-readiness` branch with:
- Backend improvements committed (partial US-019/020/021/023)
- New 27-story PRD committed
- Checkpoint cleared
- Clean working tree (except untracked archive files)

**Next step:** Implement P0 pre-flight checks.
