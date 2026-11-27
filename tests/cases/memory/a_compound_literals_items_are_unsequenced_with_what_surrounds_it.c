int h(int a, int b);

int a[2];

int stored(int *p) {
    p = a;
    p = (int *){p++};
    return 0;
}

int read_beside_a_write(int *q) {
    int x = 0;
    q = a;
    return h((int){*q}, *(q = &x));
}

int written_beside_a_read(int *q) {
    int x = 0;
    q = a;
    return h(*q, (int){*(q = &x)});
}
