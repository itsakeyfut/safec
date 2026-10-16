void *malloc(int n);
void free(void *p);
int push(int **head, int **tail) {
    if (!head) return 0;
    if (!tail) return 0;
    int *n = malloc(4);
    if (!n) return 0;
    *n = 0;
    *tail = n;
    int *h = *head;
    if (!h) return 0;
    return *h;
}
