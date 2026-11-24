struct padded {
    char tag;
    long long value;
};

int main(void) {
    int a[8];
    struct padded s[5];
    int wrong = 0;

    int *first = &a[1];
    int *last = &a[6];
    wrong += last - first == 5 ? 0 : 1;
    wrong += first - last == -5 ? 0 : 1;
    // One past the end, which C17 6.5.6 p9 allows.
    wrong += a + 8 - a == 8 ? 0 : 1;
    // Counted in elements, whatever padding an element carries.
    wrong += &s[4] - &s[1] == 3 ? 0 : 1;
    return wrong;
}
