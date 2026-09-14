void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p;
    int *keep;
    int saved = 0;
    while (c) {
        p = malloc(8);
        if (saved) {
            free(keep);
        }
        keep = p;
        saved = 1;
        free(p);
        c = c - 1;
    }
    return 0;
}
