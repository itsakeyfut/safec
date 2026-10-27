void *malloc(int n);
void free(void *p);
void stash(int **box);

int main(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    stash(box);
    int *x = malloc(4);
    if (x == 0) {
        return 1;
    }
    free(x);
    *box = x;
    return 0;
}
