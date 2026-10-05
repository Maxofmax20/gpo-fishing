"""Multitask vision model: shared CNN backbone, game-state head + entity head.

IMAGE (3x96x96, dataset-normalized; authoritative size lives in
dataset.INPUT_SIZE and each run's config.json — not here)
  -> Conv(3,32) -> BN -> ReLU -> MaxPool
  -> Conv(32,64) -> BN -> ReLU -> MaxPool
  -> Conv(64,128) -> BN -> ReLU -> MaxPool
  -> Conv(128,256) -> BN -> ReLU -> AdaptiveAvgPool (global)
  -> shared 256-d embedding (dropout in training)
  |-- Linear(256 -> 3):  GAME STATE (waiting_for_bite, bite, catch_result)
  +-- Linear(256 -> 25): ENTITY (project vocabulary, RESULT rows only)

The entity head loss is masked: only entity-linked RESULT rows carry
entity supervision. WAITING/BITE rows train the state head only.
No metadata (filenames, timestamps, sessions) ever enters the network.
"""

from __future__ import annotations

import torch
from torch import nn

STATE_LABELS = ["waiting_for_bite", "bite", "catch_result"]
# Canonical input geometry. Normalization mean/std are per-run values in
# <run>/config.json (computed from train rows), NOT 0.5. The Rust shadow
# path must read them from the deployed model manifest.
INPUT_SIZE = 96


class GpoVisionNet(nn.Module):
    def __init__(self, n_entities: int, dropout: float = 0.25) -> None:
        super().__init__()
        self.n_entities = n_entities
        def block(cin: int, cout: int) -> nn.Sequential:
            return nn.Sequential(
                nn.Conv2d(cin, cout, kernel_size=3, padding=1, bias=False),
                nn.BatchNorm2d(cout),
                nn.ReLU(inplace=True),
                nn.MaxPool2d(2),
            )

        self.backbone = nn.Sequential(
            block(3, 32),
            block(32, 64),
            block(64, 128),
            block(128, 256),
            nn.AdaptiveAvgPool2d(1),
        )
        self.dropout = nn.Dropout(dropout)
        self.state_head = nn.Linear(256, len(STATE_LABELS))
        self.entity_head = nn.Linear(256, n_entities)

    def forward(self, x: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        f = self.backbone(x).flatten(1)
        f = self.dropout(f)
        return self.state_head(f), self.entity_head(f)

    def param_count(self) -> int:
        return sum(p.numel() for p in self.parameters())
