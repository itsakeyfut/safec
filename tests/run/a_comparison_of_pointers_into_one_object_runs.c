int main(void) {
    int a[4];
    for (int i = 0; i < 4; i++) {
        a[i] = i + 1;
    }
    int *first = &a[0];
    int *last = &a[3];
    int *end = a + 4;
    int wrong = 0;

    wrong += first < last ? 0 : 1;
    wrong += last < first ? 1 : 0;
    wrong += first <= first ? 0 : 1;
    wrong += last <= first ? 1 : 0;
    wrong += last > first ? 0 : 1;
    wrong += first > last ? 1 : 0;
    wrong += last >= last ? 0 : 1;
    wrong += first >= last ? 1 : 0;

    // A walk up to one past the end, which C17 6.5.8 p5 orders too.
    int sum = 0;
    for (int *p = a; p < end; p++) {
        sum += *p;
    }
    wrong += sum == 10 ? 0 : 1;
    return wrong;
}
