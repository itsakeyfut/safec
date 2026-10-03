void *malloc(int n);
void free(void *p);
int use(int *p);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    return (free(a), 0) + use(a);
}
