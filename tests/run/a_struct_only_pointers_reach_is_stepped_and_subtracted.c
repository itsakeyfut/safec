void *malloc(unsigned long long size);
void free(void *p);

struct node {
    char tag;
    long long value;
};

static long long count(struct node *first, struct node *end) {
    return end - first;
}

static struct node *next(struct node *at) {
    at++;
    return at + 1;
}

int main(void) {
    // Allocated, so that no function here declares an object of the struct
    // and only pointers reach it.
    struct node *first = malloc(6 * sizeof(struct node));
    if (!first) {
        return 1;
    }
    int wrong = 0;
    wrong += count(first, first + 5) == 5 ? 0 : 1;
    wrong += next(first) == first + 2 ? 0 : 1;
    free(first);
    return wrong;
}
