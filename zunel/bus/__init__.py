"""Message bus module for decoupled channel-agent communication."""

from zunel.bus.events import InboundMessage, OutboundMessage
from zunel.bus.queue import MessageBus

__all__ = ["MessageBus", "InboundMessage", "OutboundMessage"]
