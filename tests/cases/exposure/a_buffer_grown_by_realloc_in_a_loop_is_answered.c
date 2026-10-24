void *realloc(void *p, int n);
void free(void *p);
int f(int n) {
    int *buf = 0;
    int *tmp = 0;
    while (n) {
        tmp = realloc(buf, 16);
        if (tmp == 0) {
            free(buf);
            return 1;
        }
        buf = tmp;
        n = n - 1;
    }
    free(buf);
    return 0;
}
