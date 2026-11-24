int one(void) {
    int a[4];
    int *end = a + 4;
    int n = 0;
    for (int *p = a; p < end; p++) {
        n++;
    }
    return n;
}

int past_its_end(void) {
    int a[2];
    return (&a[1] <= a + 2) + (a + 2 > &a[0]);
}

int apart(void) {
    int a[2];
    int b[2];
    return &a[0] < &b[0];
}

int either(int c) {
    int a[2];
    int b[2];
    int *p = c ? a : b;
    return p >= a;
}

void *malloc(unsigned long long size);

int made_each_turn(int n) {
    char *before = 0;
    int r = 0;
    for (int i = 0; i < n; i++) {
        char *p = malloc(8);
        if (!p) {
            return r;
        }
        if (before) {
            r += p < before;
        }
        before = p;
    }
    return r;
}

int read_out_of_memory(int **pp) {
    if (!pp) {
        return 0;
    }
    int *p = *pp;
    int *q = *pp;
    return p > q;
}

int one_or_another(void) {
    int a[2];
    int b[2];
    return &a[0] <= &b[0];
}

struct pair {
    int *first;
    int *second;
};

int through_fields(void) {
    int a[2];
    int b[2];
    struct pair v;
    v.first = &a[0];
    v.second = &b[0];
    return v.first < v.second;
}

int whole[4];
int other[4];

int one_static(void) {
    return &whole[0] < &whole[3];
}

int two_statics(void) {
    return &whole[0] < &other[0];
}

int maybe_null(int c) {
    int a[2];
    int *p = 0;
    if (c) {
        p = a;
    }
    return p < a + 1;
}

void release(char *p);

int maybe_freed(void) {
    char *p = malloc(8);
    if (!p) {
        return 0;
    }
    char *end = p + 8;
    release(p);
    return p < end;
}

void free(void *p);

int within_one_turn(int n) {
    int r = 0;
    for (int i = 0; i < n; i++) {
        char *p = malloc(8);
        if (!p) {
            return r;
        }
        r += p < p + 1;
        free(p);
    }
    return r;
}

struct pair_of_ints {
    int x;
    int y;
};

int members_of_one_struct(void) {
    struct pair_of_ints s;
    return &s.x < &s.y;
}

int main(void) {
    return one() + past_its_end() + apart() + either(1) + made_each_turn(2) + read_out_of_memory(0)
        + one_or_another() + through_fields() + one_static() + two_statics() + maybe_null(0)
        + maybe_freed() + within_one_turn(2) + members_of_one_struct();
}
