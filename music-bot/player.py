import random
from collections import deque
from dataclasses import dataclass


@dataclass(frozen=True)
class Track:
    id: str
    title: str
    artist: str
    album: str
    path: str

    @property
    def label(self):
        return f"{self.artist} - {self.title}" if self.artist else self.title


class Player:
    def __init__(self, rng=None):
        self.rng = rng or random.Random()
        self.queue = deque()
        self.shuffle = False
        self.deck = []
        self.current = None

    def add(self, tracks, shuffled=False):
        tracks = list(tracks)
        if shuffled:
            self.rng.shuffle(tracks)
        self.queue.extend(tracks)
        return len(tracks)

    def start_shuffle(self):
        if self.shuffle:
            return False
        self.shuffle = True
        return True

    def needs_library(self):
        return self.shuffle and not self.queue and not self.deck

    def refill(self, tracks):
        deck = list(tracks)
        self.rng.shuffle(deck)
        if len(deck) > 1 and deck[-1] == self.current:
            deck[0], deck[-1] = deck[-1], deck[0]
        self.deck = deck
        if not deck:
            self.shuffle = False

    def advance(self):
        if self.queue:
            self.current = self.queue.popleft()
        elif self.shuffle and self.deck:
            self.current = self.deck.pop()
        else:
            self.current = None
        return self.current

    def stop(self):
        self.queue.clear()
        self.shuffle = False
        self.deck = []
        self.current = None
