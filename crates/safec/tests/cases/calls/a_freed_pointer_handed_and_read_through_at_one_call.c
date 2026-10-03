void *malloc(int n);
void free(void *p);
int both(int *p, int x);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    free(a);
    return both(a, *a);
}
