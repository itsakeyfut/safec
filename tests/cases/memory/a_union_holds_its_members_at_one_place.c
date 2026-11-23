union Slot {
    int *pointer;
    long long number;
    int narrow;
};

int same_member(void) {
    int x = 1;
    union Slot s;
    s.pointer = &x;
    return *s.pointer - 1;
}

int pointer_read_as_number(void) {
    int x = 1;
    union Slot s;
    s.pointer = &x;
    int *back = (int *)s.number;
    return *back - 1;
}

int number_read_as_pointer(void) {
    union Slot s;
    s.number = 42;
    return *s.pointer;
}

int pointer_cut_short(void) {
    int x = 1;
    union Slot s;
    s.pointer = &x;
    s.narrow = 0;
    return *s.pointer;
}

int main(void) {
    return same_member() + pointer_read_as_number() + number_read_as_pointer() + pointer_cut_short();
}
