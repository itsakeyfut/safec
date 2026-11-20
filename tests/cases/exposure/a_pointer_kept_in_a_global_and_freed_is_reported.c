void *malloc(int n);
void free(void *p);
int *kept;
int main(void) {
    int *p;
    p = malloc(4);
    if (p == 0) {
        return 1;
    }
    kept = p;
    free(p);
    *kept = 1;
    return 0;
}
