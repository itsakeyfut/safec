void *malloc(int n);
void free(void *p);
void *memcpy(void *s1, void *s2, int n);
void keep(int **p);
int main(void) {
    int *slot = 0;
    keep(&slot);
    int *x = malloc(4);
    if (x == 0) {
        return 1;
    }
    free(x);
    memcpy(&slot, &x, 8);
    return 0;
}
