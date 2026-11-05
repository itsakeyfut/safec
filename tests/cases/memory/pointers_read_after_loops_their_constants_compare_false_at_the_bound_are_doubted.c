int lt_at(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i < 2) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}

int lt_past(void) {
    int x = 1;
    int *m[1];
    int i = 3;
    while (i < 2) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}

int gt_at(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i > 2) {
        m[0] = &x;
        i = i - 1;
    }
    int *q = m[0];
    return q == 0;
}

int gt_past(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i > 3) {
        m[0] = &x;
        i = i - 1;
    }
    int *q = m[0];
    return q == 0;
}

int eq(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i == 3) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}
