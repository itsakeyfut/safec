void *malloc(int n);
int f(int c) {
    int *keep = 0;
    int *made = 0;
    int n = 1;
    while (c) {
        int *p = malloc(4);
        made = keep + n;
        keep = p;
    }
    *made = 1;
    return 0;
}
