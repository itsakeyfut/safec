void *malloc(int n);
void free(void *p);

void drop(int *p) {
    free(p);
}

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    free(a);
    drop(a);
    return 0;
}
