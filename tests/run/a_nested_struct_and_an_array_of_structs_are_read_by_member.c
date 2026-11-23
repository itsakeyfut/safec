void *malloc(unsigned long long size);
void free(void *pointer);

struct Point {
    char tag;
    int x;
    int y;
};

struct Box {
    char kind;
    struct Point corner;
    struct Point many[2];
    long long area;
};

struct Node {
    struct Node *next;
    int value;
};

struct Opaque;

struct Box boxes[2];

static int sum(struct Node *list) {
    int total = 0;
    while (list) {
        total = total + list->value;
        list = list->next;
    }
    return total;
}

static struct Opaque *nothing(void) {
    return 0;
}

int main(void) {
    int r = 0;
    struct Box b;
    b.kind = 1;
    b.corner.x = 2;
    b.corner.y = 3;
    b.many[1].y = 4;
    b.area = 5;
    r = r + (b.corner.x + b.corner.y + b.many[1].y + b.area + b.kind == 15 ? 0 : 1);

    struct Point *p = &b.corner;
    p->x = 20;
    r = r + (b.corner.x == 20 ? 0 : 1);

    boxes[1].many[0].x = 7;
    r = r + (boxes[1].many[0].x == 7 ? 0 : 1);

    struct Point points[3];
    points[2].tag = 9;
    struct Point *q = points;
    r = r + (q[2].tag == 9 ? 0 : 1);

    struct Node *a = malloc(sizeof *a);
    struct Node *c = malloc(sizeof *c);
    if (!a || !c) {
        return 1;
    }
    a->value = 1;
    a->next = c;
    c->value = 2;
    c->next = 0;
    r = r + (sum(a) == 3 ? 0 : 1);
    free(c);
    free(a);

    r = r + (nothing() == 0 ? 0 : 1);
    r = r + (sizeof(struct Box) == 48 || sizeof(struct Box) == 44 ? 0 : 1);
    return r;
}
