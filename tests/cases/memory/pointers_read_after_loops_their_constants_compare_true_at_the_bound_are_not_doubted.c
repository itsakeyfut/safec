int le_at(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i <= 2) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}

int le_inside(void) {
    int x = 1;
    int *m[1];
    int i = 1;
    while (i <= 2) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}

int ge_at(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i >= 2) {
        m[0] = &x;
        i = i - 1;
    }
    int *q = m[0];
    return q == 0;
}

int ge_inside(void) {
    int x = 1;
    int *m[1];
    int i = 3;
    while (i >= 2) {
        m[0] = &x;
        i = i - 1;
    }
    int *q = m[0];
    return q == 0;
}

int ne(void) {
    int x = 1;
    int *m[1];
    int i = 2;
    while (i != 3) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q == 0;
}
