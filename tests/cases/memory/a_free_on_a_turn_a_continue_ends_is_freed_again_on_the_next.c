void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int i;
    for (i = 0; i < 2; i = i + 1) {
        free(p);
        continue;
    }
    return 0;
}
