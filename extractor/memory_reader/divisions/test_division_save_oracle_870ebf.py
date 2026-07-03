import unittest

from extractor.memory_reader.divisions import division_save_oracle_870ebf as oracle


class DivisionSaveOracleShapeTests(unittest.TestCase):
    def test_orders_group_structural_members_match_parse_oob_direct_shape(self) -> None:
        text = """
        countries={
            GER={
                theatres={
                    theatre={
                        orders_group={
                            id={ id=10 type=4713 }
                            members={
                                member={ unit={ id=111 type=4713 } }
                            }
                            member={ unit={ id=222 type=4713 } }
                        }
                    }
                }
            }
        }
        """

        structural = oracle.scan_structural_expectations(text)
        store = oracle.parse_oob.parse_oob_text(text)

        parsed_members = sum(len(army["members"]) for army in store["armies"])
        self.assertEqual(parsed_members, 1)
        self.assertEqual(structural["oob_member_unit_refs"], parsed_members)


if __name__ == "__main__":
    unittest.main()
