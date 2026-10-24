void *malloc(int n);
void free(void *p);
int g(int *p);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    return g(a) + (free(a), 0);
}
