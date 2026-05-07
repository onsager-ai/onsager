#[cfg(test)]
mod tests {
    //! Tests are written around the parked-state-machine event flow that
    //! phase 4 introduced. Tick semantics are now: emit a request event +
    //! park, then resume on a later tick once the test harness pushes a
    //! response into the parking maps. Each scenario walks the pipeline
    //! through the relevant park stages explicitly so a regression in the
    //! resume logic surfaces as a wrong event sequence rather than a
    //! silent missed advance.

    use crate::core::kernel::BaselineKernel;
    use crate::core::pipeline::*;
    use onsager_artifact::{ContentRef, Kind};
    use onsager_spine::factory_event::ShapingOutcome;

    fn fresh_pipeline() -> ForgePipeline {
        ForgePipeline::new(PendingVerdicts::new(), PendingShapings::new())
    }

    fn shaping_completed(req: &str) -> ShapingResult {
        ShapingResult {
            request_id: req.into(),
            outcome: ShapingOutcome::Completed,
            content_ref: Some(ContentRef {
                uri: "git://test@abc".into(),
                checksum: None,
            }),
            change_summary: "mock shaping completed".into(),
            quality_signals: vec![],
            session_id: "mock_session".into(),
            duration_ms: 100,
            error: None,
        }
    }

    fn shaping_failed(req: &str) -> ShapingResult {
        ShapingResult {
            request_id: req.into(),
            outcome: ShapingOutcome::Failed,
            content_ref: None,
            change_summary: "shaping failed".into(),
            quality_signals: vec![],
            session_id: "mock_session".into(),
            duration_ms: 100,
            error: Some(onsager_spine::protocol::ErrorDetail {
                code: "test_failure".into(),
                message: "mock failure".into(),
                retriable: Some(true),
            }),
        }
    }

    /// Pull the `gate_id` off whichever pre-dispatch GateRequested
    /// event the tick emitted. Tests use this to address the matching
    /// pending_verdicts entry.
    fn pre_dispatch_gate_id(output: &TickOutput) -> String {
        for ev in &output.events {
            if let PipelineEvent::GateRequested {
                gate_id,
                gate_point: GatePoint::PreDispatch,
                ..
            } = ev
            {
                return gate_id.clone();
            }
        }
        panic!("no pre-dispatch GateRequested event in tick output");
    }

    fn transition_gate_id(output: &TickOutput) -> String {
        for ev in &output.events {
            if let PipelineEvent::GateRequested {
                gate_id,
                gate_point: GatePoint::StateTransition,
                ..
            } = ev
            {
                return gate_id.clone();
            }
        }
        panic!("no state-transition GateRequested event in tick output");
    }

    fn shaping_request_id(output: &TickOutput) -> String {
        for ev in &output.events {
            if let PipelineEvent::ShapingDispatched { request_id, .. } = ev {
                return request_id.clone();
            }
        }
        panic!("no ShapingDispatched event in tick output");
    }

    /// Mock kernel that snapshots `WorldState.insights` to verify the
    /// pipeline threads insights from the shared cache (issue #36).
    struct CapturingKernel {
        seen: std::sync::Mutex<Vec<Vec<onsager_spine::protocol::Insight>>>,
    }
    impl SchedulingKernel for CapturingKernel {
        fn decide(&self, world: &WorldState) -> Option<ShapingDecision> {
            self.seen.lock().unwrap().push(world.insights.clone());
            None
        }
        fn observe(&mut self, _event: &onsager_spine::factory_event::FactoryEvent) {}
    }

    #[test]
    fn tick_feeds_insight_cache_into_world_state() {
        use onsager_spine::protocol::{FactoryEventRef, Insight};
        use onsager_spine::{InsightKind, InsightScope};

        let cache = InsightCache::default();
        cache.push(Insight {
            insight_id: "ins_1".into(),
            kind: InsightKind::Failure,
            scope: InsightScope::ArtifactKind("code".into()),
            observation: "many overrides".into(),
            evidence: vec![FactoryEventRef {
                event_id: 7,
                event_type: "forge.gate_verdict".into(),
            }],
            suggested_action: None,
            confidence: 0.8,
        });

        let mut pipeline = fresh_pipeline().with_insight_cache(cache.clone());
        pipeline.store.register(Kind::Code, "x", "marvin");

        let kernel = CapturingKernel {
            seen: Default::default(),
        };
        pipeline.tick(&kernel);

        let seen = kernel.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].len(), 1);
        assert_eq!(seen[0][0].insight_id, "ins_1");
    }

    #[test]
    fn first_tick_emits_pre_dispatch_gate_and_parks() {
        let mut pipeline = fresh_pipeline();
        pipeline.store.register(Kind::Code, "test-art", "marvin");

        let output = pipeline.tick(&BaselineKernel::new());

        // The new flow emits a single GateRequested + parks. No
        // synchronous ShapingDispatched / advance on the first tick.
        let gate_count = output
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    PipelineEvent::GateRequested {
                        gate_point: GatePoint::PreDispatch,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(gate_count, 1, "exactly one pre-dispatch gate emitted");
        assert!(
            !output
                .events
                .iter()
                .any(|e| matches!(e, PipelineEvent::ShapingDispatched { .. })),
            "shaping must not dispatch until pre-dispatch verdict arrives"
        );
        assert!(pipeline.is_parked(), "pipeline must park awaiting verdict");
    }

    #[test]
    fn parked_tick_idles_until_verdict_lands() {
        // Re-ticking while parked with no incoming verdict must emit
        // IdleTick and leave the park untouched.
        let mut pipeline = fresh_pipeline();
        pipeline.store.register(Kind::Code, "test-art", "marvin");
        pipeline.tick(&BaselineKernel::new());
        assert!(pipeline.is_parked());

        let output = pipeline.tick(&BaselineKernel::new());
        assert!(
            output
                .events
                .iter()
                .all(|e| matches!(e, PipelineEvent::IdleTick)),
            "parked tick without pending verdict must only emit IdleTick"
        );
        assert!(pipeline.is_parked(), "park survives an empty resume");
    }

    #[test]
    fn full_lifecycle_drives_artifact_to_in_progress() {
        // Three ticks: emit pre-dispatch gate, then resume after Allow
        // (emits shaping_dispatched + parks), then resume after the
        // shaping result lands (emits the transition gate + parks),
        // then a final tick after the transition verdict advances state.
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone());
        let id = pipeline.store.register(Kind::Code, "test-art", "marvin");

        // Tick 1: emit pre-dispatch gate, park.
        let out = pipeline.tick(&BaselineKernel::new());
        let gate_id = pre_dispatch_gate_id(&out);

        // Park drained: Allow.
        pending_verdicts.insert(&gate_id, GateVerdict::Allow);
        let out = pipeline.tick(&BaselineKernel::new());
        let req_id = shaping_request_id(&out);
        assert!(pipeline.is_parked());

        // Park drained: shaping result lands.
        pending_shapings.insert(&req_id, shaping_completed(&req_id));
        let out = pipeline.tick(&BaselineKernel::new());
        let transition_gate = transition_gate_id(&out);
        assert!(pipeline.is_parked(), "still parked on transition verdict");

        // Park drained: Allow on the transition gate → advance.
        pending_verdicts.insert(&transition_gate, GateVerdict::Allow);
        pipeline.tick(&BaselineKernel::new());
        assert!(!pipeline.is_parked(), "park cleared after advance");

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::InProgress);
        assert_eq!(art.current_version, 1);
    }

    #[test]
    fn pre_dispatch_deny_clears_park_and_emits_error() {
        let pending_verdicts = PendingVerdicts::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), PendingShapings::new());
        pipeline.store.register(Kind::Code, "test-art", "marvin");

        let out = pipeline.tick(&BaselineKernel::new());
        let gate_id = pre_dispatch_gate_id(&out);

        pending_verdicts.insert(
            &gate_id,
            GateVerdict::Deny {
                reason: "policy violation".into(),
            },
        );
        let out = pipeline.tick(&BaselineKernel::new());

        assert!(
            out.events
                .iter()
                .any(|e| matches!(e, PipelineEvent::Error(_))),
            "Deny on pre-dispatch must emit an error event"
        );
        assert!(!pipeline.is_parked(), "Deny clears the park");
    }

    #[test]
    fn pre_dispatch_escalate_clears_park_silently() {
        // forge invariant #5: park non-blockingly. The pipeline drops its
        // own park so the kernel can re-propose; the escalation itself
        // sits on the spine for a delegate.
        let pending_verdicts = PendingVerdicts::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), PendingShapings::new());
        pipeline.store.register(Kind::Code, "test-art", "marvin");

        let out = pipeline.tick(&BaselineKernel::new());
        let gate_id = pre_dispatch_gate_id(&out);

        pending_verdicts.insert(
            &gate_id,
            GateVerdict::Escalate {
                context: onsager_spine::protocol::EscalationContext {
                    escalation_id: "esc_1".into(),
                    reason: "human pls".into(),
                    target: "supervisor".into(),
                    timeout_at: chrono::Utc::now(),
                },
            },
        );
        pipeline.tick(&BaselineKernel::new());
        assert!(
            !pipeline.is_parked(),
            "Escalate releases the pipeline's own park"
        );
    }

    #[test]
    fn shaping_failure_clears_park_and_does_not_advance() {
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone());
        let id = pipeline.store.register(Kind::Code, "test-art", "marvin");

        let out = pipeline.tick(&BaselineKernel::new());
        let gate_id = pre_dispatch_gate_id(&out);

        pending_verdicts.insert(&gate_id, GateVerdict::Allow);
        let out = pipeline.tick(&BaselineKernel::new());
        let req_id = shaping_request_id(&out);

        pending_shapings.insert(&req_id, shaping_failed(&req_id));
        let out = pipeline.tick(&BaselineKernel::new());

        assert!(out
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::Error(_))));
        assert!(!pipeline.is_parked());
        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(
            art.state,
            ArtifactState::Draft,
            "Failed shaping must not advance the artifact"
        );
        assert_eq!(art.current_version, 0);
    }

    #[test]
    fn transition_deny_clears_park_and_does_not_advance() {
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone());
        let id = pipeline.store.register(Kind::Code, "test-art", "marvin");

        let out = pipeline.tick(&BaselineKernel::new());
        let gate_id = pre_dispatch_gate_id(&out);
        pending_verdicts.insert(&gate_id, GateVerdict::Allow);
        let out = pipeline.tick(&BaselineKernel::new());
        let req_id = shaping_request_id(&out);

        pending_shapings.insert(&req_id, shaping_completed(&req_id));
        let out = pipeline.tick(&BaselineKernel::new());
        let transition_gate = transition_gate_id(&out);

        pending_verdicts.insert(
            &transition_gate,
            GateVerdict::Deny {
                reason: "no go".into(),
            },
        );
        let out = pipeline.tick(&BaselineKernel::new());
        assert!(out
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::Error(_))));
        assert!(!pipeline.is_parked());
        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Draft);
    }

    #[test]
    fn tick_idles_when_no_work() {
        let mut pipeline = fresh_pipeline();
        let output = pipeline.tick(&BaselineKernel::new());
        assert!(output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::IdleTick)));
    }

    #[test]
    fn tick_idles_when_paused() {
        let mut pipeline = fresh_pipeline();
        pipeline.store.register(Kind::Code, "test-art", "marvin");
        pipeline
            .state
            .transition(onsager_spine::factory_event::ForgeProcessState::Paused)
            .unwrap();

        let output = pipeline.tick(&BaselineKernel::new());
        assert!(output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::IdleTick)));
        assert!(!pipeline.is_parked(), "paused tick must not park anything");
    }

    /// Drive a single decision end-to-end with Allow on both gates and a
    /// Completed shaping result. Helper for the seal tests below.
    fn drive_to_completion(
        pipeline: &mut ForgePipeline,
        pending_verdicts: &PendingVerdicts,
        pending_shapings: &PendingShapings,
        kernel: &dyn SchedulingKernel,
    ) {
        let out = pipeline.tick(kernel);
        let gate_id = pre_dispatch_gate_id(&out);
        pending_verdicts.insert(&gate_id, GateVerdict::Allow);

        let out = pipeline.tick(kernel);
        let req_id = shaping_request_id(&out);
        pending_shapings.insert(&req_id, shaping_completed(&req_id));

        let out = pipeline.tick(kernel);
        let trans = transition_gate_id(&out);
        pending_verdicts.insert(&trans, GateVerdict::Allow);

        pipeline.tick(kernel);
    }

    /// Mock SealSink: returns a deterministic bundle id per artifact.
    struct MockSeal {
        counter: std::sync::atomic::AtomicU32,
    }
    impl MockSeal {
        fn new() -> Self {
            Self {
                counter: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }
    impl SealSink for MockSeal {
        fn seal_release(
            &self,
            artifact_id: &onsager_artifact::ArtifactId,
            _result: &ShapingResult,
        ) -> Result<SealedRef, SealError> {
            let version = self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            Ok(SealedRef {
                bundle_id: ArtifactVersionId::new(format!(
                    "bnd_mock_{}_{}",
                    artifact_id.as_str(),
                    version
                )),
                version,
            })
        }
    }

    /// Kernel that always targets `Released` for any artifact in
    /// `UnderReview`.
    struct ReleaseKernel;
    impl SchedulingKernel for ReleaseKernel {
        fn decide(&self, world: &WorldState) -> Option<ShapingDecision> {
            let art = world
                .artifacts
                .iter()
                .find(|a| a.state == ArtifactState::UnderReview)?;
            Some(ShapingDecision {
                artifact_id: art.artifact_id.clone(),
                target_version: art.current_version + 1,
                target_state: ArtifactState::Released,
                shaping_intent: serde_json::Value::Null,
                inputs: vec![],
                constraints: vec![],
                deadline: None,
                priority: 100,
            })
        }
        fn observe(&mut self, _event: &onsager_spine::factory_event::FactoryEvent) {}
    }

    #[test]
    fn seal_emits_bundle_sealed_on_release() {
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone())
            .with_warehouse(Box::new(MockSeal::new()));
        let id = pipeline.store.register(Kind::Code, "svc", "marvin");

        // Drive to UnderReview with the baseline kernel (two decisions).
        let baseline = BaselineKernel::new();
        drive_to_completion(
            &mut pipeline,
            &pending_verdicts,
            &pending_shapings,
            &baseline,
        );
        drive_to_completion(
            &mut pipeline,
            &pending_verdicts,
            &pending_shapings,
            &baseline,
        );
        assert_eq!(
            pipeline.store.get(&id).unwrap().state,
            ArtifactState::UnderReview
        );

        // Now push to Released and seal — the final tick of
        // `drive_to_completion` produces the BundleSealed event.
        let release = ReleaseKernel;
        let out1 = pipeline.tick(&release);
        let g = pre_dispatch_gate_id(&out1);
        pending_verdicts.insert(&g, GateVerdict::Allow);
        let out2 = pipeline.tick(&release);
        let r = shaping_request_id(&out2);
        pending_shapings.insert(&r, shaping_completed(&r));
        let out3 = pipeline.tick(&release);
        let t = transition_gate_id(&out3);
        pending_verdicts.insert(&t, GateVerdict::Allow);
        let out4 = pipeline.tick(&release);

        let sealed_event = out4.events.iter().find_map(|e| match e {
            PipelineEvent::BundleSealed {
                artifact_id,
                bundle_id,
                version,
            } => Some((artifact_id.clone(), bundle_id.clone(), *version)),
            _ => None,
        });
        let (evt_artifact, evt_bundle, evt_version) =
            sealed_event.expect("BundleSealed event expected on release");

        assert_eq!(evt_artifact, id.to_string());
        assert_eq!(evt_version, 1);

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Released);
        assert_eq!(art.current_version_id.as_ref(), Some(&evt_bundle));
        assert_eq!(art.version_history.len(), 1);
    }

    /// SealSink that always returns a terminal sealing error.
    struct FailingSeal;
    impl SealSink for FailingSeal {
        fn seal_release(
            &self,
            _artifact_id: &onsager_artifact::ArtifactId,
            _result: &ShapingResult,
        ) -> Result<SealedRef, SealError> {
            Err(SealError::Invalid("mock seal failure".into()))
        }
    }

    #[test]
    fn seal_failure_blocks_release_transition() {
        // warehouse-and-delivery-v0.1 §5.1: Released implies a sealed
        // bundle. If sealing fails, the artifact must not advance.
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone())
            .with_warehouse(Box::new(FailingSeal));
        let id = pipeline.store.register(Kind::Code, "svc", "marvin");

        // Two complete cycles with the baseline kernel walk through
        // Draft → InProgress → UnderReview.
        let baseline = BaselineKernel::new();
        drive_to_completion(
            &mut pipeline,
            &pending_verdicts,
            &pending_shapings,
            &baseline,
        );
        drive_to_completion(
            &mut pipeline,
            &pending_verdicts,
            &pending_shapings,
            &baseline,
        );

        // Attempt the release. The transition gate Allow path tries to
        // seal, fails, and emits an error event — no advance, no
        // BundleSealed.
        let release = ReleaseKernel;
        let out1 = pipeline.tick(&release);
        let g = pre_dispatch_gate_id(&out1);
        pending_verdicts.insert(&g, GateVerdict::Allow);
        let out2 = pipeline.tick(&release);
        let r = shaping_request_id(&out2);
        pending_shapings.insert(&r, shaping_completed(&r));
        let out3 = pipeline.tick(&release);
        let t = transition_gate_id(&out3);
        pending_verdicts.insert(&t, GateVerdict::Allow);
        let out = pipeline.tick(&release);

        let has_advance = out.events.iter().any(|e| {
            matches!(
                e,
                PipelineEvent::ArtifactAdvanced {
                    to_state: ArtifactState::Released,
                    ..
                }
            )
        });
        assert!(
            !has_advance,
            "sealing failure must abort the release transition"
        );
        assert!(!out
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::BundleSealed { .. })));
        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::UnderReview);
        assert!(art.current_version_id.is_none());
    }

    #[test]
    fn release_transition_advances_without_bundle_sealed_when_warehouse_absent() {
        // Pipelines that don't attach a SealSink (legacy deployments)
        // still advance to Released — they just don't emit a
        // BundleSealed event. Regression coverage for the Allow branch
        // of advance_after_transition when self.warehouse is None.
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone());
        let id = pipeline.store.register(Kind::Code, "svc", "marvin");

        // Walk through Draft → InProgress → UnderReview with the
        // baseline kernel, then push to Released with the warehouse
        // absent.
        let baseline = BaselineKernel::new();
        drive_to_completion(
            &mut pipeline,
            &pending_verdicts,
            &pending_shapings,
            &baseline,
        );
        drive_to_completion(
            &mut pipeline,
            &pending_verdicts,
            &pending_shapings,
            &baseline,
        );
        assert_eq!(
            pipeline.store.get(&id).unwrap().state,
            ArtifactState::UnderReview
        );

        let release = ReleaseKernel;
        let out1 = pipeline.tick(&release);
        let g = pre_dispatch_gate_id(&out1);
        pending_verdicts.insert(&g, GateVerdict::Allow);
        let out2 = pipeline.tick(&release);
        let r = shaping_request_id(&out2);
        pending_shapings.insert(&r, shaping_completed(&r));
        let out3 = pipeline.tick(&release);
        let t = transition_gate_id(&out3);
        pending_verdicts.insert(&t, GateVerdict::Allow);
        let out = pipeline.tick(&release);

        // Released, no BundleSealed event.
        assert!(out.events.iter().any(|e| {
            matches!(
                e,
                PipelineEvent::ArtifactAdvanced {
                    to_state: ArtifactState::Released,
                    ..
                }
            )
        }));
        assert!(
            !out.events
                .iter()
                .any(|e| matches!(e, PipelineEvent::BundleSealed { .. })),
            "pipeline without SealSink must not emit BundleSealed"
        );

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Released);
        assert!(art.current_version_id.is_none());
    }
}
