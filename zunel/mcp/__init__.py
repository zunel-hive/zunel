"""Local MCP servers shipped with zunel.

Each subpackage under :mod:`zunel.mcp` is runnable as ``python -m zunel.mcp.<name>``
and speaks MCP over stdio. Drop the appropriate ``{command, args}`` entry into
``tools.mcpServers`` in ``~/.zunel/config.json`` to expose its tools to the agent.
"""
