"""Fixed-case completion envelope helpers, separate from product authority."""
from .plan import CASE_TESTS, CompletionError, validate_plan
__all__ = ('CASE_TESTS', 'CompletionError', 'validate_plan')
