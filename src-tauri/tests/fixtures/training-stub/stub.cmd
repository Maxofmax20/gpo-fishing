@echo off
rem Test-only stub trainer for the training E2E test (never used in production).
rem Args: %1=trainer output dir, %2=mode (good|bad), %3=onnx source to stage.
rem Writes the exact files the real pipeline writes: training_log.jsonl,
rem evaluation_test.json, config.json, dataset_snapshot.json, onnx_check.json,
rem plus a model.onnx staged from a real artifact passed by the caller.
mkdir "%~1" 2>nul
echo {"epoch": 1, "train_loss": 1.5000, "val_macro_f1": 0.5000, "lr": 0.0003, "seconds": 1}>> "%~1\training_log.jsonl"
echo {"epoch": 2, "train_loss": 1.2000, "val_macro_f1": 0.7000, "lr": 0.0003, "seconds": 2}>> "%~1\training_log.jsonl"
if "%~2"=="good" echo {"accuracy": 0.9000, "macro_f1": 0.8500, "n": 10, "per_entity": {"fish:shark": {"f1": 0.9000}, "fish:golden": {"f1": 0.8000}}}>> "%~1\evaluation_test.json"
if not "%~2"=="good" echo {"accuracy": 0.3000, "macro_f1": 0.2500, "n": 10, "per_entity": {"fish:shark": {"f1": 0.3000}, "fish:golden": {"f1": 0.2000}}}>> "%~1\evaluation_test.json"
echo {"normalization": {"mean": [0.0, 0.0, 0.0], "std": [1.0, 1.0, 1.0]}}>> "%~1\config.json"
echo {"test_sessions": 2}>> "%~1\dataset_snapshot.json"
echo {"max_abs_diff": 0.000001, "tolerance": 0.0001, "pass": true}>> "%~1\onnx_check.json"
copy /Y "%~3" "%~1\model.onnx" >nul
exit /b 0
