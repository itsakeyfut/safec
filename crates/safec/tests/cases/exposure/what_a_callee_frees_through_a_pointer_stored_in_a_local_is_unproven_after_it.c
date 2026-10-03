void *malloc(int n);
void free(void *p);
void drop_inner(int **box);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *slot;
    int **box = &slot;
    *box = p;
    drop_inner(box);
    return *p;
}
