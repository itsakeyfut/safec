void *malloc(int n);
void free(void *p);
int *give(int *p);

int main(void) {
    int *p = malloc(4);
    int *q = give(p);
    if (q != 0) {
        return *q;
    }
    return 0;
}
