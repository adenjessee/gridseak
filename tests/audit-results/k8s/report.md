# Agent report card: kubernetes

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Opus 5.5 alone was exact on 6 of 6; with GridSeak, 6 of 6. Sonnet 5.5 alone was exact on 4 of 6; with GridSeak, 6 of 6. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.16 each.

Answer key: GridSeak compiler-indexed call graph (85784 edges, 100% from the compiler index).

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 5/6 | 83% (44–97%) | 0 | 1 | 22.2 | $9.74 | $1.95 | 5.82M |
| Haiku 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 16.0 | $8.14 | $1.36 | 4.90M |
| Opus 5.5 alone | 6/6 | 100% (61–100%) | 0 | 0 | 13.2 | $3.84 | $0.64 | 2.82M |
| Opus 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 7.1 | $2.18 | $0.36 | 1.21M |
| Sonnet 5.5 alone | 4/6 | 67% (30–90%) | 2 | 0 | 8.9 | $1.95 | $0.49 | 2.16M |
| Sonnet 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 4.9 | $0.97 | $0.16 | 0.82M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.61 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.33 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.09 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.15 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.55 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.4 | $1.26 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.48 | 1.00 | 0.6 | $0.10 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.31 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.25 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.9 | $2.35 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.3 | $1.07 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.29 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.10 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.54 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.24 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.2 | $1.71 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.59 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.28 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 alone | 1 | PASS | 0.97 | 1.00 | 1.5 | $0.43 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.15 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.8 | $2.14 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 0.96 | 6.3 | $3.12 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.00 | 0.00 | 1.1 | $0.18 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.25 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 0.96 | 3.9 | $0.73 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.52 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 alone | 1 | ERROR | 0.00 | 0.00 | 6.0 | $2.39 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 3.0 | $1.78 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.1 | $1.02 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.0 | $0.41 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.0 | $1.69 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.8 | $0.88 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
