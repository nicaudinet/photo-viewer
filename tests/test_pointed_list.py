import pytest
from lib.pointed_list import PointedList


class TestConstruction:
    def test_starts_at_index_zero(self):
        pl = PointedList([1, 2, 3])
        assert pl.index == 0
        assert pl.current() == 1

    def test_empty_list_raises_assertion(self):
        with pytest.raises(AssertionError):
            PointedList([])


class TestNavigation:
    def test_next_moves_forward(self):
        pl = PointedList(["a", "b", "c"])
        assert pl.next() == "b"
        assert pl.next() == "c"

    def test_next_wraps_around(self):
        pl = PointedList([1, 2, 3])
        pl.next()  # index 1
        pl.next()  # index 2
        assert pl.next() == 1  # wraps to index 0

    def test_prev_moves_backward(self):
        pl = PointedList(["a", "b", "c"])
        pl.index = 2
        assert pl.prev() == "b"
        assert pl.prev() == "a"

    def test_prev_wraps_around(self):
        pl = PointedList([1, 2, 3])
        assert pl.prev() == 3  # wraps from 0 to last

    def test_single_element_next_stays(self):
        pl = PointedList(["only"])
        assert pl.next() == "only"
        assert pl.index == 0

    def test_single_element_prev_stays(self):
        pl = PointedList(["only"])
        assert pl.prev() == "only"
        assert pl.index == 0


class TestGoto:
    def test_goto_valid_index(self):
        pl = PointedList([10, 20, 30, 40])
        pl.goto(2)
        assert pl.current() == 30

    def test_goto_first_element(self):
        pl = PointedList([1, 2, 3])
        pl.index = 2
        pl.goto(0)
        assert pl.current() == 1

    def test_goto_last_element(self):
        pl = PointedList([1, 2, 3])
        pl.goto(2)
        assert pl.current() == 3

    def test_goto_negative_index_raises(self):
        pl = PointedList([1, 2, 3])
        with pytest.raises(AssertionError):
            pl.goto(-1)

    def test_goto_out_of_bounds_raises(self):
        pl = PointedList([1, 2, 3])
        with pytest.raises(AssertionError):
            pl.goto(3)


class TestGotoValue:
    def test_goto_value_existing(self):
        pl = PointedList(["apple", "banana", "cherry"])
        pl.goto_value("banana")
        assert pl.current() == "banana"
        assert pl.index == 1

    def test_goto_value_nonexisting_does_nothing(self):
        pl = PointedList(["a", "b", "c"])
        pl.index = 1
        pl.goto_value("not_here")
        assert pl.index == 1  # unchanged

    def test_goto_value_first_occurrence(self):
        pl = PointedList([1, 2, 1, 3])
        pl.goto_value(1)
        assert pl.index == 0  # finds first occurrence


class TestDelete:
    def test_delete_middle_element(self):
        pl = PointedList([1, 2, 3])
        pl.index = 1
        result = pl.delete()
        assert pl.list == [1, 3]
        assert result == 3  # returns new current
        assert pl.index == 1

    def test_delete_first_element(self):
        pl = PointedList([1, 2, 3])
        result = pl.delete()
        assert pl.list == [2, 3]
        assert result == 2
        assert pl.index == 0

    def test_delete_last_element_wraps_index(self):
        pl = PointedList([1, 2, 3])
        pl.index = 2
        result = pl.delete()
        assert pl.list == [1, 2]
        assert pl.index == 0  # wrapped around
        assert result == 1

    def test_delete_until_one_element(self):
        pl = PointedList([1, 2])
        pl.delete()
        assert pl.list == [2]
        assert pl.index == 0


class TestIteration:
    def test_iterates_all_elements(self):
        pl = PointedList([1, 2, 3])
        assert list(pl) == [1, 2, 3]

    def test_iteration_does_not_affect_index(self):
        pl = PointedList([1, 2, 3])
        pl.index = 1
        list(pl)  # iterate
        assert pl.index == 1  # unchanged
