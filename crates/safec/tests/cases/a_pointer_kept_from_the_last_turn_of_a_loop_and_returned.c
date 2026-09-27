void *malloc(int n);

int *previous(int n) {
    int *prev = 0;
    int *cur = 0;
    int i = 0;
    while (i < n) {
        prev = cur;
        cur = malloc(4);
        i = i + 1;
    }
    return prev;
}
