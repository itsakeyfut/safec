void *malloc(int n);
void free(void *p);
int *kept;
void drop(void) {
    free(kept);
}
int main(void) {
    int *p;
    p = malloc(4);
    if (p == 0) {
        return 1;
    }
    kept = p;
    drop();
    *p = 1;
    return 0;
}
