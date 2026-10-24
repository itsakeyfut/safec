void *malloc(int n);
void free(void *p);
void keep(int **where);
void forget(void);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *slot = 0;
    keep(&slot);
    slot = p;
    forget();
    return *p;
}
