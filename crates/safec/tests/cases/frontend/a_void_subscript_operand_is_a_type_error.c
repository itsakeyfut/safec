void g(void);

int f(int *a, int i) {
    a[g()] = 1;
    i = g()[a];
    return i;
}
