void free(void *p);

int g(int a[4]) {
    free(a);
    return a[0];
}
