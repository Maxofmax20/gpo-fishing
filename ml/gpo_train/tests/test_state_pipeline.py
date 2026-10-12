"""Exercise the real exporter and evaluation contracts without live data or training."""
import json
from types import SimpleNamespace
import numpy as np
import pytest
from PIL import Image

torch = pytest.importorskip("torch")
ort = pytest.importorskip("onnxruntime")
pytest.importorskip("onnx")
from ml.gpo_train import state_train
from ml.gpo_train.dataset import STATE_LABELS
from ml.gpo_train.model import GpoVisionNet


def test_state_pipeline_emits_single_state_head_and_frozen_evidence(tmp_path, monkeypatch):
    torch.manual_seed(7)
    out = tmp_path / "synthetic-state"
    out.mkdir()
    (out / "config.json").write_text(json.dumps({"normalization": {"mean": [0.5]*3, "std": [0.25]*3}}))
    (out / "dataset_snapshot.json").write_text(json.dumps({"entity_vocab": ["fish:a", "fish:b"], "test_sessions": 1}))
    model = GpoVisionNet(n_entities=2)
    torch.save({"model": model.state_dict()}, out / "best.pt")
    rows = []
    for i, label in enumerate(STATE_LABELS * 2):
        image = tmp_path / f"{i}.png"
        Image.new("RGB", (96, 96), (i*30, 80, 120)).save(image)
        rows.append(SimpleNamespace(image_id=str(i), image_path=str(image), game_state=label,
                                    entity_id="fish:a", session_id="val" if i < 3 else "test"))
    snapshot = SimpleNamespace(rows=rows)
    monkeypatch.setattr(state_train.train, "OUTPUT_DIR", str(tmp_path))
    monkeypatch.setattr(state_train, "load_snapshot", lambda: snapshot)
    monkeypatch.setattr(state_train, "build_splits", lambda _: SimpleNamespace(validation=[0,1,2], test=[3,4,5]))
    assert state_train.finish_run("synthetic-state") == 0
    report = json.loads((out / "evaluation_test.json").read_text())
    assert set(report["per_class"]) == set(STATE_LABELS)
    assert len(report["holdout_fingerprint"]) == 64
    assert 0 <= report["ece"] <= 1
    assert json.loads((out / "config.json").read_text())["vocab"] == STATE_LABELS
    assert json.loads((out / "onnx_check.json").read_text())["pass"] is True
    session = ort.InferenceSession(str(out / "gpo_vision_v1.onnx"), providers=["CPUExecutionProvider"])
    assert len(session.get_outputs()) == 1
    assert session.run(None, {"pixels": np.zeros((4,3,96,96), np.float32)})[0].shape == (4, len(STATE_LABELS))

    from ml.gpo_train.reevaluate_holdout import evaluate_model
    import hashlib
    artifact = out / "gpo_vision_v1.onnx"
    manifest = {"classes": STATE_LABELS, "sha256": hashlib.sha256(artifact.read_bytes()).hexdigest(),
                "temperature": json.loads((out / "calibration.json").read_text())["temperature"],
                "preprocess": {"pad": "square-black", "resize": "bilinear", "size": 96,
                               "mean": [0.5]*3, "std": [0.25]*3}}
    baseline = evaluate_model(snapshot, [3,4,5], manifest, artifact, "state")
    assert baseline["holdout_fingerprint"] == report["holdout_fingerprint"]
    assert baseline["macro_f1"] == report["macro_f1"]
    assert baseline["ece"] == pytest.approx(report["ece"], abs=1e-6)
    legacy = out / "legacy.onnx"
    torch.onnx.export(model.eval(), torch.zeros(1, 3, 96, 96), str(legacy),
                      input_names=["pixels"], output_names=["state_logits", "entity_logits"],
                      opset_version=18, dynamo=False)
    legacy_manifest = dict(manifest, sha256=hashlib.sha256(legacy.read_bytes()).hexdigest())
    legacy_report = evaluate_model(snapshot, [3,4,5], legacy_manifest, legacy, "state")
    assert legacy_report["macro_f1"] == baseline["macro_f1"]
    assert legacy_report["ece"] == pytest.approx(baseline["ece"], abs=1e-6)
    assert legacy_report["holdout_fingerprint"] == baseline["holdout_fingerprint"]
    manifest["sha256"] = "0"*64
    with pytest.raises(ValueError, match="checksum"):
        evaluate_model(snapshot, [3,4,5], manifest, artifact, "state")
