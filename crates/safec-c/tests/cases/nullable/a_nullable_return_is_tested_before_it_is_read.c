int * _Nullable find(void) {
    return 0;
}

int tested(void) {
    int *q = find();
    if (q) {
        return *q;
    }
    return 0;
}

int untested(void) {
    int *q = find();
    return *q;
}
