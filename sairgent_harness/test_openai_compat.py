import importlib.util
import pathlib
import unittest

from openai.types.chat.chat_completion import ChatCompletion, Choice
from openai.types.chat.chat_completion_message import ChatCompletionMessage
from openai.types.chat.chat_completion_message_function_tool_call import (
    ChatCompletionMessageFunctionToolCall,
    Function,
)


def _load_harness_main():
    main_path = pathlib.Path(__file__).with_name("main.py")
    spec = importlib.util.spec_from_file_location("sairgent_harness_main", main_path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class OpenAICompatTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.harness_main = _load_harness_main()

    def test_normalize_missing_finish_reason_defaults_to_stop(self):
        message = ChatCompletionMessage.model_construct(
            content="hello",
            role="assistant",
            tool_calls=None,
            function_call=None,
            refusal=None,
            annotations=None,
            audio=None,
        )
        choice = Choice.model_construct(
            finish_reason=None,
            index=0,
            logprobs=None,
            message=message,
        )
        response = ChatCompletion.model_construct(
            id="chatcmpl_test",
            choices=[choice],
            created=0,
            model="gpt-4o-mini",
            object="chat.completion",
            service_tier=None,
            system_fingerprint=None,
            usage=None,
        )

        updated = self.harness_main._normalize_openai_finish_reasons(response)

        self.assertTrue(updated)
        self.assertEqual(response.choices[0].finish_reason, "stop")

    def test_normalize_missing_finish_reason_uses_tool_calls(self):
        tool_call = ChatCompletionMessageFunctionToolCall.model_construct(
            id="call_123",
            type="function",
            function=Function.model_construct(name="queue_managed_work", arguments="{}"),
        )
        message = ChatCompletionMessage.model_construct(
            content=None,
            role="assistant",
            tool_calls=[tool_call],
            function_call=None,
            refusal=None,
            annotations=None,
            audio=None,
        )
        choice = Choice.model_construct(
            finish_reason=None,
            index=0,
            logprobs=None,
            message=message,
        )
        response = ChatCompletion.model_construct(
            id="chatcmpl_test",
            choices=[choice],
            created=0,
            model="gpt-4o-mini",
            object="chat.completion",
            service_tier=None,
            system_fingerprint=None,
            usage=None,
        )

        updated = self.harness_main._normalize_openai_finish_reasons(response)

        self.assertTrue(updated)
        self.assertEqual(response.choices[0].finish_reason, "tool_calls")


if __name__ == "__main__":
    unittest.main()
