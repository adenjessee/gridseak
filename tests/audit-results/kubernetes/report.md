# Agent report card: kubernetes

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 13 of 18; with GridSeak, 18 of 18. Opus 5.5 alone was exact on 18 of 18; with GridSeak, 18 of 18. Sonnet 5.5 alone was exact on 16 of 18; with GridSeak, 18 of 18. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.15 each.

Answer key: GridSeak compiler-indexed call graph (85784 edges, 100% from the compiler index).

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 13/18 | 72% (49–88%) | 0 | 5 | 57.2 | $25.43 | $1.96 | 14.19M |
| Haiku 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 34.5 | $16.95 | $0.94 | 9.23M |
| Opus 5.5 alone | 18/18 | 100% (82–100%) | 0 | 0 | 44.1 | $11.49 | $0.64 | 8.44M |
| Opus 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 21.0 | $5.91 | $0.33 | 2.99M |
| Sonnet 5.5 alone | 16/18 | 89% (67–97%) | 2 | 0 | 38.7 | $9.06 | $0.57 | 13.46M |
| Sonnet 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 14.9 | $2.71 | $0.15 | 2.30M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.61 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.33 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.53 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.45 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.1 | $0.48 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.8 | $0.38 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.09 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.10 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.06 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.10 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.3 | $0.05 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.15 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.17 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.15 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.17 |
| t1 (callers) | `autoConvert_v1beta1_SubjectAccessReviewStatus_To_authorization_SubjectAccessReviewStatus` (pkg/apis/authorization/v1beta1/zz_generated.conversion.go:480) | 8 | 3 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.55 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.4 | $1.26 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.63 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.23 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 3.1 | $1.32 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.0 | $0.34 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.48 | 1.00 | 0.6 | $0.10 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 alone | 1 | PASS | 0.96 | 1.00 | 0.8 | $0.17 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.06 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.7 | $0.12 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.06 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.31 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.25 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.27 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.17 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.8 | $0.22 |
| t2 (callers) | `DaemonSetsController.getDaemonPods` (pkg/controller/daemon/daemon_controller.go:761) | 23 | 5 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.17 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.9 | $2.35 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.3 | $1.07 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.8 | $1.50 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.79 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 4.4 | $2.05 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.23 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.29 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.10 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.26 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.1 | $0.24 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.09 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.54 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.24 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.46 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.20 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.5 | $0.54 |
| t3 (callers) | `WaitForDeploymentComplete` (test/e2e/framework/deployment/wait.go:45) | 32 | 3 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.17 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.2 | $1.71 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.59 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.2 | $1.66 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.6 | $0.70 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 3.5 | $1.51 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.3 | $0.64 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.28 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.31 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.1 | $0.23 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 alone | 1 | PASS | 0.97 | 1.00 | 1.5 | $0.43 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.15 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 alone | 1 | PASS | 0.97 | 1.00 | 1.9 | $0.50 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.19 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.4 | $0.46 |
| t4 (callers) | `runtimeState.setPodCIDR` (pkg/kubelet/runtime.go:110) | 36 | 8 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.14 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.8 | $2.14 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 0.96 | 6.3 | $3.12 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 alone | 1 | ERROR | 0.00 | 0.00 | 4.0 | $1.51 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.3 | $0.99 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 alone | 2 | ERROR | 0.00 | 0.00 | 3.3 | $1.33 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 2.0 | $1.04 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.00 | 0.00 | 1.1 | $0.18 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.25 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.96 | 6.8 | $1.99 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.26 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 alone | 2 | PASS | 0.97 | 0.96 | 7.1 | $2.08 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.4 | $0.27 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 0.96 | 3.9 | $0.73 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.52 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 0.96 | 5.6 | $1.18 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.49 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 alone | 2 | PASS | 1.00 | 0.96 | 7.6 | $1.32 |
| t5 (callers) | `GetPauseImageName` (test/utils/image/manifest.go:341) | 171 | 5 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.7 | $0.48 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 alone | 1 | ERROR | 0.00 | 0.00 | 6.0 | $2.39 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 3.0 | $1.78 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 alone | 1 | MISS (flagged) | 0.69 | 1.00 | 4.4 | $2.25 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.9 | $1.49 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 alone | 2 | ERROR | 0.00 | 0.00 | 2.4 | $0.93 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 2.8 | $1.52 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.1 | $1.02 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.0 | $0.41 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.9 | $0.84 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.9 | $0.33 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 4.7 | $0.68 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 2.1 | $0.35 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.0 | $1.69 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.8 | $0.88 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.1 | $1.38 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.7 | $0.74 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 4.9 | $0.98 |
| t6 (callers) | `CreateNamespaceOrDie` (test/integration/framework/util.go:34) | 324 | 4 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 3.0 | $0.72 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
