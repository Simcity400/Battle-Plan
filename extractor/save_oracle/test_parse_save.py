import unittest

from parse_save import Tokenizer, parse_countries_section


class ParseCountryCapitalIdentityTests(unittest.TestCase):
    def test_current_and_original_capitals_remain_distinct(self):
        result = {"countries": {}}
        parse_countries_section(
            Tokenizer("USA={ capital=362 original_capital=361 }"),
            result,
        )

        self.assertEqual(result["countries"]["USA"]["capital"], 362)
        self.assertEqual(result["countries"]["USA"]["original_capital"], 361)


if __name__ == "__main__":
    unittest.main()
