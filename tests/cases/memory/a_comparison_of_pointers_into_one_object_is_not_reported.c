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

int main(void) {
    return one() + past_its_end() + apart() + either(1);
}
