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

int element_number_read_as_pointer(void) {
    int x = 1;
    union Slot table[2];
    table[0].pointer = &x;
    table[0].number = 5;
    return *table[0].pointer;
}

void fill(union Slot *_Nonnull slot);
int take(union Slot *_Nonnull slot);

int handed_on(void) {
    int x = 1;
    union Slot s;
    s.pointer = &x;
    fill(&s);
    return take(&s);
}

int main(void) {
    return same_member() + pointer_read_as_number() + number_read_as_pointer() + pointer_cut_short()
        + element_number_read_as_pointer() + handed_on();
}
