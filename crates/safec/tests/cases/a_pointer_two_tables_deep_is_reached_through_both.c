void *malloc(int n);
void free(void *p);
void drop_all(int ***outer);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int **inner = malloc(8);
    if (inner == 0) {
        return 0;
    }
    int ***outer = malloc(8);
    if (outer == 0) {
        return 0;
    }
    *inner = p;
    *outer = inner;
    drop_all(outer);
    return *p;
}
