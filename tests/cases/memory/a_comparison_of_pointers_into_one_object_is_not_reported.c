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

int main(void) {
    return one() + past_its_end() + apart() + either(1) + made_each_turn(2) + read_out_of_memory(0);
}
