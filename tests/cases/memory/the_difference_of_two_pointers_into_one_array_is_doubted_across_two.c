void *malloc(unsigned long long size);
void free(void *p);

int one(void) {
    int a[4];
    return (int)(&a[3] - &a[1]);
}

int apart(void) {
    int a[4];
    int b[4];
    return (int)(&a[3] - &b[1]);
}

struct pair {
    int x;
    int y;
};

int two_members(void) {
    struct pair s;
    return (int)(&s.y - &s.x);
}

struct halves {
    int a[2];
    int b[2];
};

int two_arrays_in_one(void) {
    struct halves s;
    return (int)(&s.b[0] - &s.a[0]);
}

int in_a_block(void) {
    int *p = malloc(16);
    if (!p) {
        return 0;
    }
    int d = (int)(p + 3 - p);
    free(p);
    return d;
}

int after_a_free(void) {
    char *p = malloc(8);
    if (!p) {
        return 0;
    }
    char *q = p + 4;
    free(p);
    return (int)(q - p);
}

int maybe_null(int c) {
    int a[2];
    int *p = 0;
    if (c) {
        p = a;
    }
    return (int)(a + 1 - p);
}

int main(void) {
    return one() + apart() + two_members() + two_arrays_in_one() + in_a_block() + after_a_free()
        + maybe_null(1);
}
