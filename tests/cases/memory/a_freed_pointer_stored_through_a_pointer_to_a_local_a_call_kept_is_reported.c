void *malloc(int n);
void free(void *p);
void keep(int **where);

int main(void) {
    int *slot = 0;
    keep(&slot);
    int **ps = &slot;
    int *x = malloc(4);
    if (x == 0) {
        return 1;
    }
    free(x);
    *ps = x;
    return 0;
}
